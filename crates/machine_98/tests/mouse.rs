use common::{Bus, CpuMode, MachineModel, NoTrace};
use machine_98::Pc9801Bus;

fn make_bus() -> Pc9801Bus<NoTrace> {
    Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000)
}

/// Injects a mouse delta and flushes it into the accumulators by calling
/// sync_frame twice: once with the delta, once with zero to transfer
/// remaining into accumulator.
fn inject_and_flush_delta(bus: &mut Pc9801Bus<NoTrace>, dx: i16, dy: i16) {
    bus.push_mouse_delta(dx, dy);
    bus.push_mouse_delta(0, 0);
}

#[test]
fn mouse_default_port_a_returns_button_state() {
    let mut bus = make_bus();
    let port_a = bus.io_read_byte(0x7FD9);
    // Default: all buttons released (active-low, all high), middle forced.
    assert_eq!(port_a & 0xF0, 0xE0);
}

#[test]
fn mouse_latch_via_port_c_write() {
    let mut bus = make_bus();

    inject_and_flush_delta(&mut bus, 15, -8);

    // Write HC=0 first, then HC=1 (rising edge triggers latch).
    bus.io_write_byte(0x7FDD, 0x00);
    bus.io_write_byte(0x7FDD, 0x80);

    // Read X low nibble: HC=1, SXY=0, SHL=0
    bus.io_write_byte(0x7FDD, 0x80);
    let x_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_lo, 0x0F, "15 = 0x0F, low nibble = 0xF");

    // Read X high nibble: HC=1, SXY=0, SHL=1
    bus.io_write_byte(0x7FDD, 0xA0);
    let x_hi = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_hi, 0x00, "15 = 0x000F, high nibble = 0x0");

    // Read Y low nibble: HC=1, SXY=1, SHL=0
    // -8 as i16 = 0xFFF8, low nibble = 0x8
    bus.io_write_byte(0x7FDD, 0xC0);
    let y_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(y_lo, 0x08);

    // Read Y high nibble: HC=1, SXY=1, SHL=1
    // -8 as i16 = 0xFFF8, (value >> 4) & 0x0F = 0x0F
    bus.io_write_byte(0x7FDD, 0xE0);
    let y_hi = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(y_hi, 0x0F);
}

#[test]
fn mouse_latch_via_bsr_command() {
    let mut bus = make_bus();

    inject_and_flush_delta(&mut bus, 5, 3);

    // Clear HC via BSR: reset bit 7 -> write 0x0E (bit=7, reset)
    bus.io_write_byte(0x7FDF, 0x0E);

    // Set HC via BSR: set bit 7 -> write 0x0F (bit=7, set)
    bus.io_write_byte(0x7FDF, 0x0F);

    // Verify latch by reading X low nibble: HC=1, SXY=0, SHL=0
    bus.io_write_byte(0x7FDD, 0x80);
    let x_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_lo, 0x05);

    // Y low nibble
    bus.io_write_byte(0x7FDD, 0xC0);
    let y_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(y_lo, 0x03);
}

#[test]
fn mouse_port_c_readback() {
    let mut bus = make_bus();

    // Write port C with specific control bits (output mode = upper nibble).
    bus.io_write_byte(0x7FDD, 0xA0); // HC=1, SXY=0, SHL=1, INT#=0

    // Read port C back -- upper nibble is output (echoed from latch),
    // lower nibble is input (DIP switches: MODSW=1).
    let port_c = bus.io_read_byte(0x7FDD);
    assert_eq!(
        port_c & 0xF0,
        0xA0,
        "Upper nibble should echo written value"
    );
    assert_eq!(port_c & 0x08, 0x08, "MODSW should be set");
}

#[test]
fn mouse_button_via_port_a() {
    let mut bus = make_bus();
    bus.set_mouse_buttons(true, false, false);

    let port_a = bus.io_read_byte(0x7FD9);
    assert_eq!(port_a & 0x80, 0x00, "LEFT pressed = bit 7 clear");
    assert_eq!(port_a & 0x40, 0x40, "MIDDLE not pressed = bit 6 set");
    assert_eq!(port_a & 0x20, 0x20, "RIGHT not pressed = bit 5 set");
}

#[test]
fn mouse_latch_resets_accumulator() {
    let mut bus = make_bus();

    // Inject 10, latch, then inject 20, latch again.
    // Second latch should capture only 20, not 30.
    inject_and_flush_delta(&mut bus, 10, 0);

    // Latch via port C: HC 0->1
    bus.io_write_byte(0x7FDD, 0x00);
    bus.io_write_byte(0x7FDD, 0x80);

    // Read X low to verify first latch = 10
    bus.io_write_byte(0x7FDD, 0x80);
    let x_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_lo, 0x0A); // 10 = 0x0A

    // Inject 20 more
    inject_and_flush_delta(&mut bus, 20, 0);

    // Latch again
    bus.io_write_byte(0x7FDD, 0x00);
    bus.io_write_byte(0x7FDD, 0x80);

    // Read X low nibble of second latch
    bus.io_write_byte(0x7FDD, 0x80);
    let x_lo = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_lo, 0x04); // 20 = 0x14, low nibble = 0x4

    // Read X high nibble
    bus.io_write_byte(0x7FDD, 0xA0);
    let x_hi = bus.io_read_byte(0x7FD9) & 0x0F;
    assert_eq!(x_hi, 0x01); // 20 = 0x14, high nibble = 0x1
}

/// Unmasks only the slave cascade on the master PIC and slave IR5 (IR13).
fn unmask_only_mouse_timer(bus: &mut Pc9801Bus<NoTrace>) {
    bus.io_write_byte(0x02, 0x7F);
    bus.io_write_byte(0x0A, 0xDF);
}

/// Acknowledges the pending IR13 request and ends it on both PICs.
fn acknowledge_mouse_timer(bus: &mut Pc9801Bus<NoTrace>) {
    assert_eq!(bus.acknowledge_irq(), 0x15);
    bus.io_write_byte(0x08, 0x20);
    bus.io_write_byte(0x00, 0x20);
}

/// CPU cycles of one 120 Hz mouse timer period.
fn mouse_timer_period(bus: &Pc9801Bus<NoTrace>) -> u64 {
    u64::from(bus.cpu_clock_hz()).div_ceil(120)
}

/// undoc98 io_mouse.txt: the ITF init command 93h clears port C, which sets
/// INT# = 0, and BFDBh resets to 120 Hz. The timer interrupt runs without any
/// port C write.
#[test]
fn mouse_timer_runs_after_reset_with_int_enabled() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    unmask_only_mouse_timer(&mut bus);
    let period = mouse_timer_period(&bus);
    let start = bus.current_cycle();

    bus.set_current_cycle(start + period - 1);
    assert!(!bus.has_irq());
    for tick in 1..=3 {
        bus.set_current_cycle(start + period * tick);
        assert!(bus.has_irq(), "IR13 must be raised at tick {tick}");
        acknowledge_mouse_timer(&mut bus);
    }
}

/// undoc98 io_mouse.txt: an 8255 mode word clears port C and leaves INT# = 0.
/// The running timer keeps its period.
#[test]
fn mouse_timer_keeps_running_after_mode_word() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    unmask_only_mouse_timer(&mut bus);
    let period = mouse_timer_period(&bus);
    let start = bus.current_cycle();

    bus.set_current_cycle(start + period / 2);
    bus.io_write_byte(0x7FDF, 0x93);
    bus.set_current_cycle(start + period);
    assert!(bus.has_irq());
    acknowledge_mouse_timer(&mut bus);
    bus.set_current_cycle(start + period * 2);
    assert!(bus.has_irq());
}

/// undoc98 io_mouse.txt: INT# = 1 masks the timer interrupt and INT# = 0
/// enables it again.
#[test]
fn mouse_timer_stops_with_int_masked_and_restarts_with_int_enabled() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    unmask_only_mouse_timer(&mut bus);
    let period = mouse_timer_period(&bus);
    let start = bus.current_cycle();

    bus.set_current_cycle(start + period);
    assert!(bus.has_irq());
    acknowledge_mouse_timer(&mut bus);

    bus.io_write_byte(0x7FDF, 0x09);
    bus.set_current_cycle(start + period * 4);
    assert!(!bus.has_irq());

    let restart = bus.current_cycle();
    bus.io_write_byte(0x7FDF, 0x08);
    bus.set_current_cycle(restart + period - 1);
    assert!(!bus.has_irq());
    bus.set_current_cycle(restart + period);
    assert!(bus.has_irq());
}

/// The PC-9801VM resets port C to F0h with INT# = 1. The timer interrupt starts
/// when INT# is cleared.
#[test]
fn mouse_timer_waits_for_int_enable_after_reset_with_int_masked() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::High, 48000);
    unmask_only_mouse_timer(&mut bus);
    let period = mouse_timer_period(&bus);
    let start = bus.current_cycle();

    bus.set_current_cycle(start + period * 4);
    assert!(!bus.has_irq());

    let enable = bus.current_cycle();
    bus.io_write_byte(0x7FDF, 0x08);
    bus.set_current_cycle(enable + period);
    assert!(bus.has_irq());
    acknowledge_mouse_timer(&mut bus);
}

/// undoc98 io_mouse.txt: INT# = 1 masks the timer interrupt. A pending IR13
/// request is released when INT# changes from 0 to 1.
#[test]
fn mouse_timer_int_masked_releases_pending_request() {
    let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::High, 48000);
    unmask_only_mouse_timer(&mut bus);
    let period = mouse_timer_period(&bus);
    let start = bus.current_cycle();

    bus.set_current_cycle(start + period);
    assert!(bus.has_irq());
    bus.io_write_byte(0x7FDF, 0x09);
    assert!(!bus.has_irq());
}
