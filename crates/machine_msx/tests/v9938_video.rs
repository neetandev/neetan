use common::Bus as _;
use machine_msx::{MainBusView, MsxBus, MsxModel};

/// VDP VRAM data port.
const VDP_DATA_PORT: u16 = 0x98;
/// VDP control and status port.
const VDP_CONTROL_PORT: u16 = 0x99;
/// V9938 palette port.
const VDP_PALETTE_PORT: u16 = 0x9A;
/// V9938 indirect-register port.
const VDP_INDIRECT_PORT: u16 = 0x9B;
/// First line-interrupt event cycle for display line zero.
const FIRST_LINE_INTERRUPT_CYCLE: u64 = 8_194;
/// First vertical-blank cycle in 212-line mode.
const FIRST_212_LINE_VBLANK_CYCLE: u64 = 54_070;
/// Normal-Z80 cycles in one Japanese NTSC frame.
const FRAME_CYCLES: u64 = 59_736;
/// Command-execute flag in status register two.
const STATUS_COMMAND_EXECUTE: u8 = 0x01;
/// Transfer-ready flag in status register two.
const STATUS_TRANSFER_READY: u8 = 0x80;

/// Writes one VDP register through the control port.
fn write_register(bus: &mut MsxBus, register: u8, value: u8) {
    let mut view = MainBusView { bus };
    view.io_write_byte(VDP_CONTROL_PORT, value);
    view.io_write_byte(VDP_CONTROL_PORT, 0x80 | register);
}

/// Writes one little-endian VDP command word.
fn write_command_word(bus: &mut MsxBus, register: u8, value: u16) {
    write_register(bus, register, value as u8);
    write_register(bus, register + 1, (value >> 8) as u8);
}

/// Selects a CPU VRAM address for reading or writing.
fn select_vram_address(bus: &mut MsxBus, address: u32, write: bool) {
    write_register(bus, 14, (address >> 14) as u8);
    let mut view = MainBusView { bus };
    view.io_write_byte(VDP_CONTROL_PORT, address as u8);
    view.io_write_byte(
        VDP_CONTROL_PORT,
        ((address >> 8) as u8 & 0x3F) | if write { 0x40 } else { 0 },
    );
    if !write {
        let end = bus.current_cycle() + 12;
        run_bus_to(bus, end);
    }
}

/// Writes bytes through the CPU VRAM port.
fn write_vram(bus: &mut MsxBus, address: u32, values: &[u8]) {
    select_vram_address(bus, address, true);
    let mut view = MainBusView { bus };
    for value in values {
        view.io_write_byte(VDP_DATA_PORT, *value);
        let end = view.bus.current_cycle() + 12;
        run_bus_to(view.bus, end);
    }
}

/// Reads one buffered byte through the CPU VRAM port.
fn read_vram(bus: &mut MsxBus, address: u32) -> u8 {
    select_vram_address(bus, address, false);
    MainBusView { bus }.io_read_byte(VDP_DATA_PORT)
}

/// Reads one selected V9938 status register.
fn read_status(bus: &mut MsxBus, status: u8) -> u8 {
    write_register(bus, 15, status);
    MainBusView { bus }.io_read_byte(VDP_CONTROL_PORT)
}

/// Advances a bus and processes every event through one cycle.
fn run_bus_to(bus: &mut MsxBus, end: u64) {
    while bus.current_cycle() < end {
        let next = bus.next_event_cycle().unwrap_or(end).min(end);
        bus.set_current_cycle(next);
        bus.process_events();
    }
}

#[test]
/// MSX2 exposes the complete V9938 I/O and 128 KiB VRAM address space.
fn ports_and_extended_vram_are_visible_only_on_v99x8_models() {
    let mut msx = MsxBus::new(MsxModel::Msx, 48_000);
    let mut msx_view = MainBusView { bus: &mut msx };
    assert_eq!(msx_view.io_read_byte(VDP_PALETTE_PORT), 0xFF);
    assert_eq!(msx_view.io_read_byte(VDP_INDIRECT_PORT), 0xFF);

    let mut msx2 = MsxBus::new(MsxModel::Msx2, 48_000);
    assert_eq!(msx2.display_dimensions(), (568, 240));
    write_register(&mut msx2, 0, 0x06);
    write_register(&mut msx2, 1, 0x40);
    write_vram(&mut msx2, 0x1FFFF, &[0xA5, 0x5A]);
    assert_eq!(read_vram(&mut msx2, 0x1FFFF), 0xA5);
    assert_eq!(read_vram(&mut msx2, 0), 0x5A);

    write_register(&mut msx2, 17, 14);
    MainBusView { bus: &mut msx2 }.io_write_byte(VDP_INDIRECT_PORT, 7);
    write_vram(&mut msx2, 0x1C000, &[0x3C]);
    assert_eq!(read_vram(&mut msx2, 0x1C000), 0x3C);
}

#[test]
/// Enabling a line interrupt after its phase waits until the next frame.
fn late_line_interrupt_enable_does_not_consume_a_disabled_event() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    write_register(&mut bus, 19, 0);
    run_bus_to(&mut bus, FIRST_LINE_INTERRUPT_CYCLE);
    assert!(!bus.has_irq());
    assert_eq!(read_status(&mut bus, 1) & 1, 0);

    write_register(&mut bus, 0, 0x10);
    assert!(!bus.has_irq());
    run_bus_to(&mut bus, FIRST_LINE_INTERRUPT_CYCLE + FRAME_CYCLES - 1);
    assert!(!bus.has_irq());
    run_bus_to(&mut bus, FIRST_LINE_INTERRUPT_CYCLE + FRAME_CYCLES);
    assert!(bus.has_irq());
    assert_eq!(read_status(&mut bus, 1) & 1, 1);
    assert!(!bus.has_irq());
}

#[test]
/// The 212-line VBlank status can assert an IRQ after a late enable.
fn vertical_interrupt_uses_the_212_line_boundary() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    write_register(&mut bus, 0, 0x06);
    write_register(&mut bus, 9, 0x80);
    run_bus_to(&mut bus, FIRST_212_LINE_VBLANK_CYCLE - 1);
    assert_eq!(bus.vdp_status() & 0x80, 0);

    run_bus_to(&mut bus, FIRST_212_LINE_VBLANK_CYCLE);
    assert_eq!(bus.vdp_status() & 0x80, 0x80);
    assert!(!bus.has_irq());
    write_register(&mut bus, 1, 0x20);
    assert!(bus.has_irq());
    assert_eq!(read_status(&mut bus, 0) & 0x80, 0x80);
    assert!(!bus.has_irq());
}

#[test]
/// Commands progress over time while CPU VRAM access remains available.
fn command_execution_is_asynchronous_during_cpu_vram_access() {
    let mut bus = MsxBus::new(MsxModel::Msx2, 48_000);
    write_register(&mut bus, 0, 0x06);
    write_register(&mut bus, 1, 0x40);
    write_command_word(&mut bus, 36, 0);
    write_command_word(&mut bus, 38, 0);
    write_command_word(&mut bus, 40, 4);
    write_command_word(&mut bus, 42, 1);
    write_register(&mut bus, 44, 0x0A);
    write_register(&mut bus, 46, 0xC0);
    assert_eq!(
        read_status(&mut bus, 2) & (STATUS_COMMAND_EXECUTE | STATUS_TRANSFER_READY),
        STATUS_COMMAND_EXECUTE
    );

    write_vram(&mut bus, 0x100, &[0x5C]);
    run_bus_to(&mut bus, 100);
    assert_eq!(
        read_status(&mut bus, 2) & (STATUS_COMMAND_EXECUTE | STATUS_TRANSFER_READY),
        0
    );
    assert_eq!(read_vram(&mut bus, 0), 0xAA);
    assert_eq!(read_vram(&mut bus, 1), 0xAA);
    assert_eq!(read_vram(&mut bus, 0x100), 0x5C);
}
