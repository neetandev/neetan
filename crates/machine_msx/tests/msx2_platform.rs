use common::{Bus as _, CpuZ80 as _};
use machine_msx::{MainBusView, MsxBus, MsxMachine, MsxModel};

/// S1985 switched-I/O selector port.
const SWITCHED_IO_SELECT_PORT: u16 = 0x40;
/// S1985 backup-RAM address port.
const S1985_BACKUP_ADDRESS_PORT: u16 = 0x41;
/// S1985 backup-RAM data port.
const S1985_BACKUP_DATA_PORT: u16 = 0x42;
/// S1985 switched-I/O device identifier.
const S1985_DEVICE_ID: u8 = 0xFE;
/// RP5C01 address-latch port.
const RTC_ADDRESS_PORT: u16 = 0xB4;
/// RP5C01 data port.
const RTC_DATA_PORT: u16 = 0xB5;
/// S1985-mirrored RP5C01 address-latch port.
const RTC_MIRROR_ADDRESS_PORT: u16 = 0xB6;
/// S1985-mirrored RP5C01 data port.
const RTC_MIRROR_DATA_PORT: u16 = 0xB7;
/// First memory-mapper register port.
const MAPPER_PAGE_ZERO_PORT: u16 = 0xFC;
/// Sony printer status port.
const PRINTER_STATUS_PORT: u16 = 0x90;
/// Sony system flags port.
const SYSTEM_FLAGS_PORT: u16 = 0xF4;

fn host_time() -> common::HostDateTime {
    common::HostDateTime {
        year: 2000,
        month: 2,
        day: 28,
        day_of_week: 1,
        hour: 23,
        minute: 59,
        second: 59,
    }
}

fn io_write(bus: &mut MsxBus, port: u16, value: u8) {
    let mut view = MainBusView { bus };
    view.io_write_byte(port, value);
}

fn io_read(bus: &mut MsxBus, port: u16) -> u8 {
    let mut view = MainBusView { bus };
    view.io_read_byte(port)
}

#[test]
fn rtc_ports_are_mirrored_and_absent_from_msx1() {
    let mut msx1 = MsxBus::new(MsxModel::Msx, 48_000);
    assert_eq!(io_read(&mut msx1, RTC_DATA_PORT), 0xFF);

    let mut msx2 = MsxBus::new(MsxModel::Msx2, 48_000);
    msx2.set_host_date_time_provider(host_time);
    io_write(&mut msx2, RTC_ADDRESS_PORT, 0);
    assert_eq!(io_read(&mut msx2, RTC_DATA_PORT), 0xF9);
    io_write(&mut msx2, RTC_MIRROR_ADDRESS_PORT, 1);
    assert_eq!(io_read(&mut msx2, RTC_MIRROR_DATA_PORT), 0xF5);
    assert_eq!(io_read(&mut msx2, RTC_ADDRESS_PORT), 0xFF);
}

#[test]
fn s1985_selection_id_and_backup_ram_are_visible() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    assert_eq!(io_read(&mut bus, SWITCHED_IO_SELECT_PORT), 0xFF);
    io_write(&mut bus, SWITCHED_IO_SELECT_PORT, S1985_DEVICE_ID);
    assert_eq!(io_read(&mut bus, SWITCHED_IO_SELECT_PORT), 0x01);
    io_write(&mut bus, S1985_BACKUP_ADDRESS_PORT, 3);
    io_write(&mut bus, S1985_BACKUP_DATA_PORT, 0xA5);
    assert_eq!(io_read(&mut bus, S1985_BACKUP_DATA_PORT), 0xA5);
    io_write(&mut bus, SWITCHED_IO_SELECT_PORT, 0x11);
    assert_eq!(io_read(&mut bus, S1985_BACKUP_DATA_PORT), 0xFF);
}

#[test]
fn sony_mapper_targets_select_the_first_and_last_512_kib_segments() {
    for model in [MsxModel::Msx2, MsxModel::Msx2Plus] {
        let mut bus = MsxBus::new(model, 48_000);
        bus.load_synthetic_program(&[0x76]).unwrap();
        bus.poke_byte(0, 0xA5);
        io_write(&mut bus, MAPPER_PAGE_ZERO_PORT, 0x3F);
        assert_eq!(io_read(&mut bus, MAPPER_PAGE_ZERO_PORT), 0x9F);
        assert_eq!(bus.peek_byte(0), 0);
        bus.poke_byte(0, 0x5A);
        io_write(&mut bus, MAPPER_PAGE_ZERO_PORT, 0);
        assert_eq!(bus.peek_byte(0), 0xA5);
        io_write(&mut bus, MAPPER_PAGE_ZERO_PORT, 0x1F);
        assert_eq!(bus.peek_byte(0), 0x5A);
    }
}

#[test]
fn hbf1xdj_mirrors_devices_and_exposes_sony_status_ports() {
    let mut bus = MsxBus::new(MsxModel::Msx2Plus, 48_000);
    io_write(&mut bus, 0x9D, 0x40);
    io_write(&mut bus, 0x9D, 0x81);
    assert_eq!(bus.vdp_render_state().register(1), 0x40);

    io_write(&mut bus, 0xA4, 7);
    io_write(&mut bus, 0xA5, 0x80);
    io_write(&mut bus, 0xA4, 14);
    assert_eq!(io_read(&mut bus, 0xA6) & 0x40, 0x40);

    assert_eq!(io_read(&mut bus, PRINTER_STATUS_PORT), 0x02);
    assert_eq!(io_read(&mut bus, PRINTER_STATUS_PORT + 1), 0xFF);
    assert_eq!(io_read(&mut bus, PRINTER_STATUS_PORT + 4), 0x02);
    assert_eq!(io_read(&mut bus, SYSTEM_FLAGS_PORT), 0);
    io_write(&mut bus, SYSTEM_FLAGS_PORT, 0xA0);
    io_write(&mut bus, SYSTEM_FLAGS_PORT, 0x80);
    assert_eq!(io_read(&mut bus, SYSTEM_FLAGS_PORT), 0xA0);
    io_write(&mut bus, SYSTEM_FLAGS_PORT, 0);
    assert_eq!(io_read(&mut bus, SYSTEM_FLAGS_PORT), 0x20);
}

#[test]
fn synthetic_program_accesses_rtc_and_s1985() {
    let program = [
        0x3E,
        S1985_DEVICE_ID,
        0xD3,
        SWITCHED_IO_SELECT_PORT as u8,
        0xDB,
        SWITCHED_IO_SELECT_PORT as u8,
        0x32,
        0x00,
        0xC0,
        0x3E,
        0x03,
        0xD3,
        S1985_BACKUP_ADDRESS_PORT as u8,
        0x3E,
        0xA5,
        0xD3,
        S1985_BACKUP_DATA_PORT as u8,
        0xDB,
        S1985_BACKUP_DATA_PORT as u8,
        0x32,
        0x01,
        0xC0,
        0xAF,
        0xD3,
        RTC_ADDRESS_PORT as u8,
        0xDB,
        RTC_DATA_PORT as u8,
        0x32,
        0x02,
        0xC0,
        0x76,
    ];
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    bus.set_host_date_time_provider(host_time);
    bus.load_synthetic_program(&program).unwrap();
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);
    machine.run_for(1_000);
    assert!(machine.main_cpu.halted());
    assert_eq!(machine.bus.peek_byte(0xC000), 0x01);
    assert_eq!(machine.bus.peek_byte(0xC001), 0xA5);
    assert_eq!(machine.bus.peek_byte(0xC002), 0xF9);
}

#[test]
fn halted_run_loop_advances_rtc_and_video_events() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    bus.set_host_date_time_provider(host_time);
    bus.load_synthetic_program(&[0x76]).unwrap();
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);
    let scanlines_before = machine.bus.completed_scanlines();
    machine.run_for(u64::from(machine.bus.cpu_clock_hz()));
    io_write(&mut machine.bus, RTC_ADDRESS_PORT, 0);
    assert_eq!(io_read(&mut machine.bus, RTC_DATA_PORT), 0xF0);
    io_write(&mut machine.bus, RTC_ADDRESS_PORT, 2);
    assert_eq!(io_read(&mut machine.bus, RTC_DATA_PORT), 0xF0);
    assert!(machine.bus.completed_scanlines() > scanlines_before);
}
