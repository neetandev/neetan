use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

const OUTPUT_SAMPLE_RATE: u32 = 48_000;

const PPI_PORT_A: u16 = 0x0088;
const PPI_PORT_B: u16 = 0x008A;
const PPI_PORT_C: u16 = 0x008C;
const PPI_MODE: u16 = 0x008E;
const ENABLE_MASK: u16 = 0x0188;
const ENABLE_MASK_MIRROR: u16 = 0x018A;
const PIT_COUNTER: u16 = 0x018C;
const PIT_CONTROL: u16 = 0x018E;

const ALL_PORTS: &[u16] = &[
    PPI_PORT_A,
    PPI_PORT_B,
    PPI_PORT_C,
    PPI_MODE,
    ENABLE_MASK,
    ENABLE_MASK_MIRROR,
    PIT_COUNTER,
    PIT_CONTROL,
];

fn setup_14_bus(model: MachineModel) -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::<NoTrace>::new(model, CpuMode::High, OUTPUT_SAMPLE_RATE);
    bus.install_soundboard_14();
    bus
}

fn expected_wait(model: MachineModel) -> i64 {
    let cbus = (u64::from(model.cpu_clock_hz(CpuMode::High)) * 6).div_ceil(10_000_000) as i64;
    1 + cbus
}

#[test]
fn soundboard_14_cbus_wait_read_20mhz() {
    let mut bus = setup_14_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);
    for port in ALL_PORTS {
        let _ = bus.io_read_byte(*port);
        assert_eq!(bus.drain_wait_cycles(), expected, "read port {port:#06X}");
    }
}

#[test]
fn soundboard_14_cbus_wait_read_66mhz() {
    let mut bus = setup_14_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);
    for port in ALL_PORTS {
        let _ = bus.io_read_byte(*port);
        assert_eq!(bus.drain_wait_cycles(), expected, "read port {port:#06X}");
    }
}

#[test]
fn soundboard_14_cbus_wait_write_20mhz() {
    let mut bus = setup_14_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);
    for port in ALL_PORTS {
        bus.io_write_byte(*port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "write port {port:#06X}");
    }
}

#[test]
fn soundboard_14_cbus_wait_write_66mhz() {
    let mut bus = setup_14_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);
    for port in ALL_PORTS {
        bus.io_write_byte(*port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "write port {port:#06X}");
    }
}

#[test]
fn soundboard_14_read_dip_switch_returns_0x08() {
    let mut bus = setup_14_bus(MachineModel::PC9801RA);
    bus.drain_wait_cycles();
    assert_eq!(bus.io_read_byte(PPI_MODE), 0x08);
}

#[test]
fn soundboard_14_read_strap_returns_0x80() {
    let mut bus = setup_14_bus(MachineModel::PC9801RA);
    bus.drain_wait_cycles();
    assert_eq!(bus.io_read_byte(PIT_CONTROL), 0x80);
}

#[test]
fn soundboard_14_enable_mask_round_trips() {
    let mut bus = setup_14_bus(MachineModel::PC9801RA);
    bus.io_write_byte(ENABLE_MASK, 0x5A);
    bus.drain_wait_cycles();
    assert_eq!(bus.io_read_byte(ENABLE_MASK), 0x5A);
    // Port 0x018A is a mirror; it should return the same value.
    assert_eq!(bus.io_read_byte(ENABLE_MASK_MIRROR), 0x5A);
}
