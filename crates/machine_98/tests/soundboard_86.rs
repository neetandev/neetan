use common::{Bus, CpuMode, MachineModel};
use machine_98::{NoTracing, Pc9801Bus};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;

const OPNA_ADDR_LO: u16 = 0x0188;
const OPNA_DATA_LO: u16 = 0x018A;
const OPNA_ADDR_HI: u16 = 0x018C;
const OPNA_DATA_HI: u16 = 0x018E;

fn setup_86_bus(model: MachineModel) -> Pc9801Bus<NoTracing> {
    let mut bus = Pc9801Bus::<NoTracing>::new(model, CpuMode::High, OUTPUT_SAMPLE_RATE);
    bus.install_soundboard_86(None, true);
    bus
}

fn expected_wait(model: MachineModel) -> i64 {
    let cbus = (u64::from(model.cpu_clock_hz(CpuMode::High)) * 6).div_ceil(10_000_000) as i64;
    1 + cbus
}

#[test]
fn soundboard_86_cbus_wait_opna_read_20mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    for port in [OPNA_ADDR_LO, OPNA_DATA_LO, OPNA_ADDR_HI, OPNA_DATA_HI] {
        bus.io_read_byte(port);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_opna_read_66mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    for port in [OPNA_ADDR_LO, OPNA_DATA_LO, OPNA_ADDR_HI, OPNA_DATA_HI] {
        bus.io_read_byte(port);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_opna_write_20mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    for port in [OPNA_ADDR_LO, OPNA_DATA_LO, OPNA_ADDR_HI, OPNA_DATA_HI] {
        bus.io_write_byte(port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_opna_write_66mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    for port in [OPNA_ADDR_LO, OPNA_DATA_LO, OPNA_ADDR_HI, OPNA_DATA_HI] {
        bus.io_write_byte(port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_pcm86_read_20mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    for port in [0xA460u16, 0xA466, 0xA46E, 0xA66E] {
        bus.io_read_byte(port);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_pcm86_read_66mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    for port in [0xA460u16, 0xA466, 0xA46E, 0xA66E] {
        bus.io_read_byte(port);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_pcm86_write_20mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9801RA);
    let expected = expected_wait(MachineModel::PC9801RA);

    for port in [0xA460u16, 0xA468, 0xA46C, 0xA66E] {
        bus.io_write_byte(port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}

#[test]
fn soundboard_86_cbus_wait_pcm86_write_66mhz() {
    let mut bus = setup_86_bus(MachineModel::PC9821AP);
    let expected = expected_wait(MachineModel::PC9821AP);

    for port in [0xA460u16, 0xA468, 0xA46C, 0xA66E] {
        bus.io_write_byte(port, 0x00);
        assert_eq!(bus.drain_wait_cycles(), expected, "port {port:#06X}");
    }
}
