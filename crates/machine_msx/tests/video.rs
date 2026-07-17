use common::Bus as _;
use machine_msx::{MainBusView, MsxBus, MsxMachine, MsxModel};

/// VDP VRAM data port.
const VDP_DATA_PORT: u16 = 0x98;
/// VDP control and status port.
const VDP_CONTROL_PORT: u16 = 0x99;
/// First active NTSC scanline latch cycle.
const FIRST_ACTIVE_LATCH_CYCLE: u64 = 8_194;
/// First vertical-blank event cycle.
const FIRST_VBLANK_CYCLE: u64 = 51_790;
/// CPU cycles in one Japanese NTSC frame.
const FRAME_CYCLES: u64 = 59_736;
/// Width of the physically visible framebuffer.
const FRAMEBUFFER_WIDTH: usize = 284;

/// Writes one TMS control register through the MSX ports.
fn write_register(bus: &mut MsxBus, register: u8, value: u8) {
    let mut view = MainBusView { bus };
    view.io_write_byte(VDP_CONTROL_PORT, value);
    view.io_write_byte(VDP_CONTROL_PORT, 0x80 | register);
}

/// Writes bytes through the TMS VRAM data port.
fn write_vram(bus: &mut MsxBus, address: u16, values: &[u8]) {
    let mut view = MainBusView { bus };
    view.io_write_byte(VDP_CONTROL_PORT, address as u8);
    view.io_write_byte(VDP_CONTROL_PORT, 0x40 | (address >> 8) as u8);
    for value in values {
        view.io_write_byte(VDP_DATA_PORT, *value);
        let end = view.bus.current_cycle() + 12;
        run_bus_to(view.bus, end);
    }
}

/// Reads TMS status register zero.
fn read_status(bus: &mut MsxBus) -> u8 {
    MainBusView { bus }.io_read_byte(VDP_CONTROL_PORT)
}

/// Advances the bus and processes every event through the requested cycle.
fn run_bus_to(bus: &mut MsxBus, end: u64) {
    while bus.current_cycle() < end {
        let next = bus.next_event_cycle().unwrap_or(end).min(end);
        bus.set_current_cycle(next);
        bus.process_events();
    }
}

/// Returns one packed framebuffer pixel.
fn pixel(bus: &MsxBus, x: usize, y: usize) -> [u8; 4] {
    let start = (y * FRAMEBUFFER_WIDTH + x) * 4;
    bus.display_framebuffer()[start..start + 4]
        .try_into()
        .unwrap()
}

/// Configures a one-cell Graphics 1 fixture.
fn configure_graphics_one(bus: &mut MsxBus) {
    write_register(bus, 0, 0);
    write_register(bus, 1, 0x40);
    write_register(bus, 2, 6);
    write_register(bus, 3, 0x20);
    write_register(bus, 4, 0);
    write_register(bus, 5, 0x20);
    write_register(bus, 6, 1);
    write_register(bus, 7, 4);
    write_vram(bus, 0x1800, &[1]);
    write_vram(bus, 0x0800, &[0xF2]);
}

#[test]
/// VDP ports expose buffered reads, wrapping and status side effects.
fn vdp_ports_expose_read_ahead_and_status_side_effects() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    write_vram(&mut bus, 0x3FFF, &[0xAA, 0x55]);
    let mut view = MainBusView { bus: &mut bus };
    view.io_write_byte(VDP_CONTROL_PORT, 0xFF);
    view.io_write_byte(VDP_CONTROL_PORT, 0x3F);
    let end = view.bus.current_cycle() + 12;
    run_bus_to(view.bus, end);
    assert_eq!(view.io_read_byte(VDP_DATA_PORT), 0xAA);
    let end = view.bus.current_cycle() + 12;
    run_bus_to(view.bus, end);
    assert_eq!(view.io_read_byte(VDP_DATA_PORT), 0x55);

    write_register(view.bus, 1, 0x20);
    run_bus_to(view.bus, FIRST_VBLANK_CYCLE);
    assert!(view.bus.has_irq());
    assert_eq!(read_status(view.bus) & 0x80, 0x80);
    assert!(!view.bus.has_irq());
}

#[test]
/// The presented surface includes the physically visible analog borders.
fn physical_border_and_active_area_are_presented() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    configure_graphics_one(&mut bus);
    write_vram(&mut bus, 8, &[0x80]);
    run_bus_to(&mut bus, FRAME_CYCLES);

    assert_eq!(bus.display_dimensions(), (284, 240));
    assert_eq!(bus.frame_number(), 1);
    assert_eq!(pixel(&bus, 0, 0), [84, 85, 237, 0xFF]);
    assert_eq!(pixel(&bus, 13, 17), [84, 85, 237, 0xFF]);
    assert_eq!(pixel(&bus, 14, 17), [255, 255, 255, 0xFF]);
    assert_eq!(pixel(&bus, 15, 17), [33, 200, 66, 0xFF]);
    assert_eq!(pixel(&bus, 283, 239), [84, 85, 237, 0xFF]);
}

#[test]
/// A VRAM write after the scanline latch cannot change the latched line.
fn vram_write_after_hblank_latch_waits_for_the_next_frame() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    configure_graphics_one(&mut bus);
    run_bus_to(&mut bus, FIRST_ACTIVE_LATCH_CYCLE);
    write_vram(&mut bus, 8, &[0x80]);
    run_bus_to(&mut bus, FRAME_CYCLES);
    assert_eq!(pixel(&bus, 14, 17), [33, 200, 66, 0xFF]);

    run_bus_to(&mut bus, FRAME_CYCLES * 2);
    assert_eq!(pixel(&bus, 14, 17), [255, 255, 255, 0xFF]);
}

#[test]
/// A real Z80 IM 1 program receives and clears vertical interrupts.
fn synthetic_program_receives_im1_vblank_interrupts() {
    let mut program = vec![0; 0x42];
    program[..10].copy_from_slice(&[0xF3, 0x31, 0x00, 0xF0, 0xED, 0x56, 0xFB, 0x76, 0x18, 0xFD]);
    program[0x38..0x42]
        .copy_from_slice(&[0x3A, 0x00, 0xC0, 0x3C, 0x32, 0x00, 0xC0, 0xDB, 0x99, 0xFB]);
    program.push(0xC9);

    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    bus.load_synthetic_program(&program).unwrap();
    write_register(&mut bus, 1, 0x20);
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = MsxMachine::new(main_cpu, bus);
    machine.run_for(FRAME_CYCLES * 2);
    assert!(machine.bus.peek_byte(0xC000) >= 1);
}

#[test]
/// The first Japanese NTSC vertical interrupt begins on the exact CPU cycle.
fn vertical_interrupt_starts_at_the_documented_cycle() {
    let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
    write_register(&mut bus, 1, 0x20);
    run_bus_to(&mut bus, FIRST_VBLANK_CYCLE - 1);
    assert!(!bus.has_irq());
    assert_eq!(bus.vdp_status() & 0x80, 0);

    run_bus_to(&mut bus, FIRST_VBLANK_CYCLE);
    assert!(bus.has_irq());
    assert_eq!(bus.vdp_status() & 0x80, 0x80);
}
