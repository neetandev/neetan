use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::{Pc9801Bus, Pc9801Vx};

const OUTPUT_SAMPLE_RATE: u32 = 48_000;

const OPN_ADDR: u16 = 0x0188;
const OPN_DATA: u16 = 0x018A;

const MEMORY_SWITCH_4: u32 = 0xA3FEE;
const MEMORY_SWITCH_4_EXPANSION_ROM_CC000: u8 = 0x08;
const MEMORY_SWITCH_WRITE_ENABLE: u8 = 0x0D;
const MEMORY_SWITCH_WRITE_DISABLE: u8 = 0x0C;

fn setup_26k_bus(model: MachineModel) -> Pc9801Bus<NoTrace> {
    let mut bus = Pc9801Bus::<NoTrace>::new(model, CpuMode::High, OUTPUT_SAMPLE_RATE);
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

#[test]
fn memory_switch_4_declares_no_expansion_rom_without_soundboard() {
    let mut bus =
        Pc9801Bus::<NoTrace>::new(MachineModel::PC9801F, CpuMode::High, OUTPUT_SAMPLE_RATE);
    assert_eq!(bus.read_byte(MEMORY_SWITCH_4), 0x00);
}

#[test]
fn soundboard_26k_declares_sound_rom_in_memory_switch_4() {
    let mut bus = setup_26k_bus(MachineModel::PC9801F);
    assert_eq!(
        bus.read_byte(MEMORY_SWITCH_4),
        MEMORY_SWITCH_4_EXPANSION_ROM_CC000
    );
}

#[test]
fn soundboard_26k_sound_rom_declaration_survives_hle_cold_reset() {
    let mut machine = Pc9801Vx::new(cpu::I286::new(), setup_26k_bus(MachineModel::PC9801VX));

    machine.bus.io_write_byte(0x68, MEMORY_SWITCH_WRITE_ENABLE);
    machine.bus.write_byte(MEMORY_SWITCH_4, 0x00);
    machine.bus.io_write_byte(0x68, MEMORY_SWITCH_WRITE_DISABLE);
    assert_eq!(machine.bus.read_byte(MEMORY_SWITCH_4), 0x00);

    #[rustfmt::skip]
    let code: &[u8] = &[
        0xB0, 0x0F,       // MOV AL, 0x0F
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x0B,       // MOV AL, 0x0B
        0xE6, 0x37,       // OUT 0x37, AL
        0xB0, 0x00,       // MOV AL, 0x00
        0xE6, 0xF0,       // OUT 0xF0, AL
        0xF4,             // HLT
    ];
    for (offset, &byte) in code.iter().enumerate() {
        machine.bus.write_byte(0x0100 + offset as u32, byte);
    }
    machine.cpu.load_state(&{
        let mut state = cpu::I286State::default();
        state.set_sp(0x1000);
        state.ip = 0x0100;
        state
    });

    machine.run_for(50_000);

    assert_eq!(
        machine.bus.read_byte(MEMORY_SWITCH_4),
        MEMORY_SWITCH_4_EXPANSION_ROM_CC000
    );
}
