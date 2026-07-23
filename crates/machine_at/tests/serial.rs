//! Bus-level tests for the COM1 INS8250/16450 UART, driven through the I/O
//! ports with placeholder ROMs (the CPU never runs). These cover the register
//! file, the divisor latch, the loopback modem reflection and the IRQ 4 path
//! through the PIC. The serial mouse attached to COM1 is exercised separately
//! in `serial_gameport.rs`.

use common::{Bus, NoTrace};
use machine_at::{AtBus, LoadedRoms};

// COM1 UART register ports.
const COM1_DATA: u16 = 0x03F8; // THR/RBR, or DLL when DLAB is set.
const COM1_IER: u16 = 0x03F9; // IER, or DLM when DLAB is set.
const COM1_LCR: u16 = 0x03FB;
const COM1_MCR: u16 = 0x03FC;
const COM1_LSR: u16 = 0x03FD;
const COM1_MSR: u16 = 0x03FE;
const COM1_SCRATCH: u16 = 0x03FF;

// Line status bits.
const LSR_DR: u8 = 0x01;
const LSR_THRE: u8 = 0x20;
const LSR_TEMT: u8 = 0x40;

// Line control bits.
const LCR_DLAB: u8 = 0x80;
const LCR_8N1: u8 = 0x03;

// Modem control bits.
const MCR_OUT1: u8 = 0x04;
const MCR_OUT2: u8 = 0x08;
const MCR_LOOP: u8 = 0x10;

// Modem status level bits.
const MSR_RI: u8 = 0x40;
const MSR_DCD: u8 = 0x80;

// Interrupt enable bits.
const IER_ETBEI: u8 = 0x02;

/// Builds a bus with placeholder ROMs and a 1.152 MHz clock.
fn bus() -> AtBus<NoTrace> {
    let roms = LoadedRoms {
        system_bios: vec![0xFF; 0x1_0000],
        vga_bios: vec![0xFF; 0x8000],
        hle: false,
    };
    AtBus::<NoTrace>::new(1_152_000, 16 << 20, roms, 48_000)
}

/// Initializes the master PIC and unmasks only IRQ 4 (COM1).
fn initialize_pic_for_com1(bus: &mut AtBus<NoTrace>) {
    bus.io_write_byte(0x20, 0x11);
    bus.io_write_byte(0x21, 0x08);
    bus.io_write_byte(0x21, 0x04);
    bus.io_write_byte(0x21, 0x01);
    bus.io_write_byte(0xA0, 0x11);
    bus.io_write_byte(0xA1, 0x70);
    bus.io_write_byte(0xA1, 0x02);
    bus.io_write_byte(0xA1, 0x01);
    bus.io_write_byte(0x21, !0x10);
    bus.io_write_byte(0xA1, 0xFF);
}

#[test]
fn reset_reports_transmitter_empty() {
    let mut bus = bus();
    let lsr = bus.io_read_byte(COM1_LSR);
    assert_ne!(lsr & LSR_THRE, 0, "THRE should be set at reset");
    assert_ne!(lsr & LSR_TEMT, 0, "TEMT should be set at reset");
    assert_eq!(lsr & LSR_DR, 0, "no received data at reset");
}

#[test]
fn scratch_register_round_trips() {
    let mut bus = bus();
    bus.io_write_byte(COM1_SCRATCH, 0xA5);
    assert_eq!(bus.io_read_byte(COM1_SCRATCH), 0xA5);
    bus.io_write_byte(COM1_SCRATCH, 0x5A);
    assert_eq!(bus.io_read_byte(COM1_SCRATCH), 0x5A);
}

#[test]
fn divisor_latch_gates_register_zero() {
    let mut bus = bus();
    // With DLAB set, register 0/1 are the divisor latch low/high bytes.
    bus.io_write_byte(COM1_LCR, LCR_DLAB);
    bus.io_write_byte(COM1_DATA, 0x0C);
    bus.io_write_byte(COM1_IER, 0x00);
    assert_eq!(
        bus.io_read_byte(COM1_DATA),
        0x0C,
        "DLL reads back the divisor"
    );
    // Clearing DLAB exposes the receive buffer at register 0 again, which reads
    // empty here rather than the divisor value.
    bus.io_write_byte(COM1_LCR, LCR_8N1);
    assert_ne!(
        bus.io_read_byte(COM1_DATA),
        0x0C,
        "register 0 is no longer DLL"
    );
}

#[test]
fn loopback_reflects_modem_control_outputs() {
    let mut bus = bus();
    // Loopback with OUT1 and OUT2 asserted loops them to the ring-indicator and
    // data-carrier-detect status inputs; clear-to-send and data-set-ready stay
    // low because RTS and DTR are not driven.
    bus.io_write_byte(COM1_MCR, MCR_LOOP | MCR_OUT1 | MCR_OUT2);
    let msr = bus.io_read_byte(COM1_MSR);
    assert_eq!(
        msr & 0xF0,
        MSR_RI | MSR_DCD,
        "only RI and DCD should be set"
    );
}

#[test]
fn transmitter_empty_interrupt_raises_irq4() {
    let mut bus = bus();
    initialize_pic_for_com1(&mut bus);
    assert!(!bus.has_irq(), "no interrupt before the UART is armed");

    // OUT2 gates the UART interrupt onto the bus; enabling the transmitter
    // holding register empty interrupt asserts immediately since THRE is set.
    bus.io_write_byte(COM1_MCR, MCR_OUT2);
    bus.io_write_byte(COM1_IER, IER_ETBEI);
    assert!(bus.has_irq(), "the THR-empty interrupt should raise IRQ 4");

    // Masking IRQ 4 at the PIC clears the pending line.
    bus.io_write_byte(0x21, 0xFF);
    assert!(!bus.has_irq(), "masking IRQ 4 clears the pending interrupt");
}
