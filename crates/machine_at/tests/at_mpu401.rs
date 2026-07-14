//! Bus-level tests for the AT MPU-401 wiring: the data/status/command port
//! decode at 0x330/0x331, the UART-mode enter and leave handshake, and IRQ 9
//! routing through the cascaded PIC. The intelligent-mode protocol itself is
//! covered by the device unit tests.

use common::Bus;
use machine_at::{AtBus, LoadedRoms};

const CPU_CLOCK_HZ: u32 = 50_000_000;

const MPU_DATA: u16 = 0x0330;
const MPU_STATUS: u16 = 0x0331;
const MPU_COMMAND: u16 = 0x0331;

/// MPU-401 acknowledge byte.
const ACK: u8 = 0xFE;
/// Status bit 7 (DRR): set when no data is available to read.
const STATUS_EMPTY: u8 = 0x80;

fn build_bus() -> AtBus {
    let roms = LoadedRoms {
        system_bios: vec![0u8; 0x1_0000],
        vga_bios: vec![0u8; 0x8000],
    };
    AtBus::new(CPU_CLOCK_HZ, 16 * 1024 * 1024, roms, 48_000)
}

/// Initializes both PICs and unmasks IRQ 9 (slave IRQ 1 behind the IRQ 2 cascade).
fn initialize_pic_for_mpu(bus: &mut AtBus) {
    bus.io_write_byte(0x20, 0x11);
    bus.io_write_byte(0x21, 0x08);
    bus.io_write_byte(0x21, 0x04);
    bus.io_write_byte(0x21, 0x01);
    bus.io_write_byte(0xA0, 0x11);
    bus.io_write_byte(0xA1, 0x70);
    bus.io_write_byte(0xA1, 0x02);
    bus.io_write_byte(0xA1, 0x01);
    bus.io_write_byte(0x21, !0x04); // unmask the cascade (IRQ 2)
    bus.io_write_byte(0xA1, !0x02); // unmask IRQ 9 (slave IRQ 1)
}

#[test]
fn at_mpu401_reset_acknowledges() {
    let mut bus = build_bus();
    bus.io_write_byte(MPU_COMMAND, 0xFF); // reset
    assert_eq!(bus.io_read_byte(MPU_STATUS) & STATUS_EMPTY, 0x00);
    assert_eq!(bus.io_read_byte(MPU_DATA), ACK);
    assert_eq!(bus.io_read_byte(MPU_STATUS) & STATUS_EMPTY, STATUS_EMPTY);
}

#[test]
fn at_mpu401_enters_and_leaves_uart_mode() {
    let mut bus = build_bus();

    // Enter UART mode: the command acknowledges.
    bus.io_write_byte(MPU_COMMAND, 0x3F);
    assert_eq!(bus.io_read_byte(MPU_DATA), ACK);

    // A MIDI byte written to the data port is swallowed by the passthrough and
    // produces no response.
    bus.io_write_byte(MPU_DATA, 0x90);
    assert_eq!(bus.io_read_byte(MPU_STATUS) & STATUS_EMPTY, STATUS_EMPTY);

    // Reset leaves UART mode and acknowledges.
    bus.io_write_byte(MPU_COMMAND, 0xFF);
    assert_eq!(bus.io_read_byte(MPU_DATA), ACK);
}

#[test]
fn at_mpu401_data_and_command_ports_are_distinct() {
    let mut bus = build_bus();

    // A write to the data port in intelligent idle produces no acknowledge.
    bus.io_write_byte(MPU_DATA, 0x90);
    assert_eq!(bus.io_read_byte(MPU_STATUS) & STATUS_EMPTY, STATUS_EMPTY);

    // A command to the command port does acknowledge.
    bus.io_write_byte(MPU_COMMAND, 0xFF);
    assert_eq!(bus.io_read_byte(MPU_STATUS) & STATUS_EMPTY, 0x00);
    assert_eq!(bus.io_read_byte(MPU_DATA), ACK);
}

#[test]
fn at_mpu401_command_routes_irq9() {
    let mut bus = build_bus();
    initialize_pic_for_mpu(&mut bus);
    assert!(!bus.has_irq(), "no IRQ pending before the command");

    bus.io_write_byte(MPU_COMMAND, 0xFF); // reset raises the MPU interrupt
    assert!(bus.has_irq(), "the MPU command should raise IRQ 9");
}
