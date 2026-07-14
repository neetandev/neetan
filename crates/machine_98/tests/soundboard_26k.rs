use common::{Bus, CpuMode, MachineModel};
use machine_98::{NoTracing, Pc9801Bus};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;

const OPN_ADDR: u16 = 0x0188;
const OPN_DATA: u16 = 0x018A;

fn setup_26k_bus(model: MachineModel) -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::<NoTracing>::new(model, CpuMode::High, OUTPUT_SAMPLE_RATE);
    bus.install_soundboard_26k(false);
    bus
}

fn expected_wait(model: MachineModel) -> i64 {
    let cbus = (u64::from(model.cpu_clock_hz(CpuMode::High)) * 6).div_ceil(10_000_000) as i64;
    1 + cbus
}

#[test]
fn soundboard_26k_cbus_wait_read_20mhz() {
    let mut bus = setup_26k_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    bus.io_read_byte(OPN_ADDR);
    assert_eq!(bus.drain_wait_cycles(), expected);

    bus.io_read_byte(OPN_DATA);
    assert_eq!(bus.drain_wait_cycles(), expected);
}

#[test]
fn soundboard_26k_cbus_wait_read_66mhz() {
    let mut bus = setup_26k_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    bus.io_read_byte(OPN_ADDR);
    assert_eq!(bus.drain_wait_cycles(), expected);

    bus.io_read_byte(OPN_DATA);
    assert_eq!(bus.drain_wait_cycles(), expected);
}

#[test]
fn soundboard_26k_cbus_wait_write_20mhz() {
    let mut bus = setup_26k_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    bus.io_write_byte(OPN_ADDR, 0x00);
    assert_eq!(bus.drain_wait_cycles(), expected);

    bus.io_write_byte(OPN_DATA, 0x00);
    assert_eq!(bus.drain_wait_cycles(), expected);
}

#[test]
fn soundboard_26k_cbus_wait_write_66mhz() {
    let mut bus = setup_26k_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    bus.io_write_byte(OPN_ADDR, 0x00);
    assert_eq!(bus.drain_wait_cycles(), expected);

    bus.io_write_byte(OPN_DATA, 0x00);
    assert_eq!(bus.drain_wait_cycles(), expected);
}
