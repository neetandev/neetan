//! Unit tests for the INS8250 / 16450 UART.

use device::ins8250_uart::{Ins8250Uart, UartWriteEffect};

// Register offsets.
const RBR_THR_DLL: u8 = 0;
const IER_DLM: u8 = 1;
const IIR: u8 = 2;
const LCR: u8 = 3;
const MCR: u8 = 4;
const LSR: u8 = 5;
const MSR: u8 = 6;
const SCRATCH: u8 = 7;

// Selected register bits used by the tests.
const LCR_DLAB: u8 = 0x80;
const IER_ERBFI: u8 = 0x01;
const IER_ETBEI: u8 = 0x02;
const IER_EDSSI: u8 = 0x08;
const MCR_RTS: u8 = 0x02;
const MCR_OUT2: u8 = 0x08;
const MCR_LOOP: u8 = 0x10;
const LSR_DR: u8 = 0x01;
const IIR_NONE: u8 = 0x01;
const IIR_MODEM_STATUS: u8 = 0x00;
const IIR_THR_EMPTY: u8 = 0x02;
const IIR_RX_DATA: u8 = 0x04;
const MSR_DELTA_CTS: u8 = 0x01;
const MSR_CTS: u8 = 0x10;

/// A UART whose core clock makes one 7-bit frame at 115200 baud last 70 cycles.
fn uart() -> Ins8250Uart {
    Ins8250Uart::new(1_152_000)
}

#[test]
fn divisor_latch_round_trips_and_sets_baud() {
    let mut uart = uart();
    uart.write(LCR, LCR_DLAB);
    uart.write(RBR_THR_DLL, 96);
    uart.write(IER_DLM, 0);
    assert_eq!(uart.read(RBR_THR_DLL), 96);
    assert_eq!(uart.read(IER_DLM), 0);
    assert_eq!(uart.baud(), 1200);
}

#[test]
fn loopback_transmit_returns_to_receiver() {
    let mut uart = uart();
    uart.write(MCR, MCR_LOOP | MCR_OUT2);
    uart.write(IER_DLM, IER_ERBFI);
    uart.write(RBR_THR_DLL, 0x41);
    assert_ne!(uart.read(LSR) & LSR_DR, 0);
    assert_eq!(uart.read(IIR), IIR_RX_DATA);
    assert!(uart.irq_asserted());
    assert_eq!(uart.read(RBR_THR_DLL), 0x41);
    // The receiver buffer is empty again.
    assert_eq!(uart.read(LSR) & LSR_DR, 0);
}

#[test]
fn loopback_modem_status_interrupt() {
    let mut uart = uart();
    uart.write(MCR, MCR_LOOP | MCR_OUT2);
    uart.write(IER_DLM, IER_EDSSI);
    // Clear the deltas produced by turning OUT2 on.
    uart.read(MSR);
    // Raising RTS drives CTS in loopback, setting the delta-CTS interrupt.
    uart.write(MCR, MCR_LOOP | MCR_OUT2 | MCR_RTS);
    assert!(uart.irq_asserted());
    assert_eq!(uart.read(IIR), IIR_MODEM_STATUS);
    let status = uart.read(MSR);
    assert_ne!(status & MSR_DELTA_CTS, 0);
    assert_ne!(status & MSR_CTS, 0);
    // Reading the modem status register cleared the delta and the interrupt.
    assert!(!uart.irq_asserted());
}

#[test]
fn transmitter_empty_interrupt_clears_on_iir_read() {
    let mut uart = uart();
    uart.write(MCR, MCR_OUT2);
    uart.write(IER_DLM, IER_ETBEI);
    assert_eq!(uart.read(IIR), IIR_THR_EMPTY);
    // Reading the identification register cleared the transmit interrupt.
    assert_eq!(uart.read(IIR), IIR_NONE);
}

#[test]
fn out2_gates_the_interrupt_line() {
    let mut uart = uart();
    uart.write(IER_DLM, IER_ERBFI);
    uart.queue_received_bytes(&[0x55], 0);
    uart.advance_to(uart.byte_duration_cycles());
    assert_ne!(uart.read(LSR) & LSR_DR, 0);
    // The source is pending but OUT2 is clear, so the line stays low.
    assert!(!uart.irq_asserted());
    uart.write(MCR, MCR_OUT2);
    assert!(uart.irq_asserted());
}

#[test]
fn received_bytes_pace_out_one_frame_apart() {
    let mut uart = uart();
    uart.write(MCR, MCR_OUT2);
    uart.write(IER_DLM, IER_ERBFI);
    let frame = uart.byte_duration_cycles();
    assert_eq!(frame, 70);
    uart.queue_received_bytes(&[0x11, 0x22, 0x33], 0);
    assert_eq!(uart.next_event_cycle(), Some(frame));

    uart.advance_to(frame);
    assert!(uart.irq_asserted());
    assert_eq!(uart.read(RBR_THR_DLL), 0x11);
    assert_eq!(uart.next_event_cycle(), Some(2 * frame));

    uart.advance_to(2 * frame);
    assert_eq!(uart.read(RBR_THR_DLL), 0x22);

    uart.advance_to(3 * frame);
    assert_eq!(uart.read(RBR_THR_DLL), 0x33);
    assert_eq!(uart.next_event_cycle(), None);
}

#[test]
fn modem_control_write_reports_edges() {
    let mut uart = uart();
    let effect = uart.write(MCR, MCR_RTS | MCR_OUT2);
    match effect {
        UartWriteEffect::ModemControlChanged {
            rts, rts_rose, dtr, ..
        } => {
            assert!(rts);
            assert!(rts_rose);
            assert!(!dtr);
        }
        UartWriteEffect::None => panic!("expected a modem control effect"),
    }
    // Holding RTS high produces no fresh rising edge.
    let effect = uart.write(MCR, MCR_RTS | MCR_OUT2);
    assert_eq!(
        effect,
        UartWriteEffect::ModemControlChanged {
            dtr: false,
            rts: true,
            dtr_rose: false,
            rts_rose: false,
        }
    );
}

#[test]
fn scratch_register_round_trips() {
    let mut uart = uart();
    uart.write(SCRATCH, 0xA5);
    assert_eq!(uart.read(SCRATCH), 0xA5);
}
