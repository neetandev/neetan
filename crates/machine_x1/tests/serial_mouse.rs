//! Z80 SIO tests (turbo): the RS-232C receiver on channel 0 and the mouse on
//! channel 1.

mod harness;

use harness::build_machine;
use machine_x1::X1Model;

/// SIO channel 0 (RS-232C) data and control ports.
const SIO_CH0_DATA: u16 = 0x1F90;
const SIO_CH0_CONTROL: u16 = 0x1F91;

/// SIO channel 1 (mouse) data and control ports.
const SIO_CH1_DATA: u16 = 0x1F92;
const SIO_CH1_CONTROL: u16 = 0x1F93;

/// RR0 status bit: a received character is available.
const RR0_RX_AVAILABLE: u8 = 0x01;

/// Points channel 1 at register `pointer` and writes `value` into it.
fn sio_ch1_write(bus: &mut machine_x1::X1Bus, pointer: u8, value: u8) {
    bus.io_write(SIO_CH1_CONTROL, pointer);
    bus.io_write(SIO_CH1_CONTROL, value);
}

/// Arms channel 1 for receive interrupts (int on all rx chars, status affects
/// vector) with the shared vector `vector`.
fn arm_mouse_interrupt(bus: &mut machine_x1::X1Bus, vector: u8) {
    sio_ch1_write(bus, 0x02, vector); // WR2: interrupt vector (channel B holds it)
    sio_ch1_write(bus, 0x01, 0x1C); // WR1: rx int on all chars + status affects vector
}

/// Drives channel 1's RTS output high-to-low, the edge that latches a mouse
/// report.
fn pulse_mouse_rts(bus: &mut machine_x1::X1Bus) {
    sio_ch1_write(bus, 0x05, 0x02); // WR5: set RTS (drives the output low)
}

#[test]
fn mouse_report_is_delivered_through_sio_channel_1() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    arm_mouse_interrupt(bus, 0x30);
    bus.set_mouse_input(5, -3, 0x01); // move, left button down

    assert!(!bus.has_irq());
    pulse_mouse_rts(bus);

    // The receive interrupt is pending and vectors through channel B with the
    // status folded in (rx-available -> affect 2 -> bits 3:1 = 0b010).
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x34);

    // The three report bytes clock out of the channel 1 data port: status
    // (left button, no overflow), dx, dy.
    assert_eq!(bus.io_read(SIO_CH1_DATA), 0x01);
    assert_eq!(bus.io_read(SIO_CH1_DATA), 5);
    assert_eq!(bus.io_read(SIO_CH1_DATA), (-3i8) as u8);
}

#[test]
fn reading_the_report_clears_the_receive_interrupt() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    arm_mouse_interrupt(bus, 0x00);
    bus.set_mouse_input(1, 1, 0x00);
    pulse_mouse_rts(bus);
    assert!(bus.has_irq());

    // Draining all three bytes clears the receive interrupt.
    let _ = bus.io_read(SIO_CH1_DATA);
    let _ = bus.io_read(SIO_CH1_DATA);
    let _ = bus.io_read(SIO_CH1_DATA);
    assert!(!bus.has_irq());
}

#[test]
fn each_rts_edge_refreshes_the_report() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    arm_mouse_interrupt(bus, 0x00);

    bus.set_mouse_input(10, 0, 0x00);
    pulse_mouse_rts(bus);
    // Release RTS then pulse again after fresh movement; the buffer is flushed
    // and reloaded, so the second report reflects only the new delta.
    sio_ch1_write(bus, 0x05, 0x00); // clear RTS
    bus.set_mouse_input(20, 0, 0x00);
    pulse_mouse_rts(bus);

    let _status = bus.io_read(SIO_CH1_DATA);
    assert_eq!(bus.io_read(SIO_CH1_DATA), 20);
}

#[test]
fn rs232c_receive_via_io() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Enable the channel-0 receiver: point WR3 and set rx-enable + 8 bits/char.
    bus.io_write(SIO_CH0_CONTROL, 0x03);
    bus.io_write(SIO_CH0_CONTROL, 0xC1);

    // No data yet: RR0 reports no received character.
    assert_eq!(bus.io_read(SIO_CH0_CONTROL) & RR0_RX_AVAILABLE, 0);

    // Inject a byte as if it arrived on the serial line.
    bus.push_rs232c_received_byte(0x41);
    assert_eq!(
        bus.io_read(SIO_CH0_CONTROL) & RR0_RX_AVAILABLE,
        RR0_RX_AVAILABLE
    );
    assert_eq!(bus.io_read(SIO_CH0_DATA), 0x41);

    // The FIFO drained: RR0 clears the received-character bit again.
    assert_eq!(bus.io_read(SIO_CH0_CONTROL) & RR0_RX_AVAILABLE, 0);
}

#[test]
fn rs232c_receive_interrupt_vectors_through_channel_a() {
    let mut machine = build_machine(X1Model::X1Turbo);
    let bus = &mut machine.bus;

    // Shared vector lives in channel B (WR2), with status-affects-vector set.
    sio_ch1_write(bus, 0x02, 0x00);
    sio_ch1_write(bus, 0x01, 0x04);
    // Channel 0: rx interrupt on all received characters.
    bus.io_write(SIO_CH0_CONTROL, 0x01);
    bus.io_write(SIO_CH0_CONTROL, 0x18);

    assert!(!bus.has_irq());
    bus.push_rs232c_received_byte(0x55);

    // Channel A receive available: affect = 4 | 2 = 6, so bits 3:1 = 0b110 = 0x0C.
    assert!(bus.has_irq());
    assert_eq!(bus.acknowledge_irq(), 0x0C);

    // Reading the byte clears the receive interrupt.
    assert_eq!(bus.io_read(SIO_CH0_DATA), 0x55);
    assert!(!bus.has_irq());
}

#[test]
fn base_x1_has_no_sio() {
    // The base X1 has no SIO: the channel ports read back as open bus.
    let mut machine = build_machine(X1Model::X1);
    let bus = &mut machine.bus;
    assert_eq!(bus.io_read(SIO_CH1_DATA), 0xFF);
    assert_eq!(bus.io_read(SIO_CH1_CONTROL), 0xFF);
}
