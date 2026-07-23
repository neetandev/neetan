//! INT 14h serial port services over the COM1 UART.

use common::Bus;

use super::{RESULT, boot_and_run, create_machine_dx50, inject_and_run, read_ram_u8, read_ram_u16};

/// FLAGS bit 0: carry.
const FLAGS_CARRY: u16 = 0x0001;
/// LSR transmitter bits: holding register and shift register empty.
const LSR_TRANSMITTER_EMPTY: u8 = 0x60;
/// COM1 line control register port.
const COM1_LCR: u16 = 0x3FB;
/// COM1 divisor latch low port (DLAB set).
const COM1_DIVISOR_LOW: u16 = 0x3F8;
/// COM1 divisor latch high port (DLAB set).
const COM1_DIVISOR_HIGH: u16 = 0x3F9;

/// AH=00h AL=E3h (9600 baud, 8N1) on COM1: stores the returned AX.
#[rustfmt::skip]
const INIT_9600_CODE: &[u8] = &[
    0xB8, 0xE3, 0x00,       // MOV AX, 0x00E3
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// AH=00h AL=43h (300 baud, 8N1) on COM1: stores the returned AX.
#[rustfmt::skip]
const INIT_300_CODE: &[u8] = &[
    0xB8, 0x43, 0x00,       // MOV AX, 0x0043
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// Initializes COM1, enables UART loopback (MCR = 0x1F), sends 0x5A and
/// receives it back: stores the send status and the receive AX.
#[rustfmt::skip]
const LOOPBACK_ROUND_TRIP_CODE: &[u8] = &[
    0xB8, 0xE3, 0x00,       // MOV AX, 0x00E3 (init 9600 8N1)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xB0, 0x1F,             // MOV AL, 0x1F (loopback + DTR/RTS/OUT1/OUT2)
    0xBA, 0xFC, 0x03,       // MOV DX, 0x3FC (MCR)
    0xEE,                   // OUT DX, AL
    0xB8, 0x5A, 0x01,       // MOV AX, 0x015A (AH=01h send, AL=0x5A)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0x88, 0x26, 0x00, 0x06, // MOV [0x0600], AH (send status)
    0xB4, 0x02,             // MOV AH, 0x02 (receive)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX (receive status and data)
    0xF4,                   // HLT
];

/// Initializes COM1, enables loopback, then AH=03h: stores the status AX.
#[rustfmt::skip]
const LOOPBACK_STATUS_CODE: &[u8] = &[
    0xB8, 0xE3, 0x00,       // MOV AX, 0x00E3 (init 9600 8N1)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xB0, 0x1F,             // MOV AL, 0x1F (loopback + DTR/RTS/OUT1/OUT2)
    0xBA, 0xFC, 0x03,       // MOV DX, 0x3FC (MCR)
    0xEE,                   // OUT DX, AL
    0xB4, 0x03,             // MOV AH, 0x03 (status)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0xF4,                   // HLT
];

/// Without loopback nothing drives the modem lines: receive and send both
/// report the timeout bit. Stores both status bytes.
#[rustfmt::skip]
const UNCONNECTED_TIMEOUT_CODE: &[u8] = &[
    0xB8, 0xE3, 0x00,       // MOV AX, 0x00E3 (init 9600 8N1)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0xB4, 0x02,             // MOV AH, 0x02 (receive, no data, no DSR)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0x88, 0x26, 0x00, 0x06, // MOV [0x0600], AH
    0xB8, 0x41, 0x01,       // MOV AX, 0x0141 (send without CTS/DSR)
    0xBA, 0x00, 0x00,       // MOV DX, 0
    0xCD, 0x14,             // INT 14h
    0x88, 0x26, 0x01, 0x06, // MOV [0x0601], AH
    0xF4,                   // HLT
];

/// AH=02h on COM4 (no BDA base address): AX must return untouched.
#[rustfmt::skip]
const INVALID_PORT_CODE: &[u8] = &[
    0xB8, 0xA5, 0x02,       // MOV AX, 0x02A5
    0xBA, 0x03, 0x00,       // MOV DX, 3
    0xCD, 0x14,             // INT 14h
    0xA3, 0x00, 0x06,       // MOV [0x0600], AX
    0x9C, 0x58,             // PUSHF; POP AX
    0xA3, 0x02, 0x06,       // MOV [0x0602], AX
    0xF4,                   // HLT
];

/// Reads the COM1 divisor latch through the guest-visible ports.
fn read_divisor(machine: &mut machine_at::AtMachine<common::NoTrace>) -> u16 {
    let line_control = machine.bus.io_read_byte(COM1_LCR);
    machine.bus.io_write_byte(COM1_LCR, line_control | 0x80);
    let divisor = u16::from(machine.bus.io_read_byte(COM1_DIVISOR_LOW))
        | (u16::from(machine.bus.io_read_byte(COM1_DIVISOR_HIGH)) << 8);
    machine.bus.io_write_byte(COM1_LCR, line_control);
    divisor
}

#[test]
fn init_programs_the_uart_divisor() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, INIT_9600_CODE, &[], 1_000_000);

    let status = read_ram_u16(&machine, RESULT);
    assert_eq!(
        (status >> 8) as u8 & LSR_TRANSMITTER_EMPTY,
        LSR_TRANSMITTER_EMPTY,
        "AH reports the transmitter empty"
    );
    assert_eq!(read_divisor(&mut machine), 0x000C, "9600 baud divisor");
    assert_eq!(
        machine.bus.io_read_byte(COM1_LCR),
        0x03,
        "LCR: 8 data bits, one stop bit, no parity"
    );

    inject_and_run(&mut machine, INIT_300_CODE, &[], 1_000_000);
    assert_eq!(read_divisor(&mut machine), 0x0180, "300 baud divisor");
}

#[test]
fn loopback_send_receive_round_trip() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, LOOPBACK_ROUND_TRIP_CODE, &[], 2_000_000);

    assert_eq!(
        read_ram_u8(&machine, RESULT) & 0x80,
        0,
        "send completed without timeout"
    );
    assert_eq!(
        read_ram_u8(&machine, RESULT + 2),
        0x5A,
        "received the sent byte"
    );
    assert_eq!(
        read_ram_u8(&machine, RESULT + 3) & 0x9E,
        0,
        "receive status: no timeout, no line errors"
    );
}

#[test]
fn status_reports_lsr_and_msr() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, LOOPBACK_STATUS_CODE, &[], 1_000_000);

    let status = read_ram_u16(&machine, RESULT);
    assert_eq!(
        (status >> 8) as u8,
        LSR_TRANSMITTER_EMPTY,
        "AH: transmitter empty, no data ready"
    );
    assert_eq!(
        status as u8 & 0xF0,
        0xF0,
        "AL: CTS, DSR, RI and DCD reflected from the loopback MCR"
    );
}

#[test]
fn unconnected_port_times_out() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, UNCONNECTED_TIMEOUT_CODE, &[], 2_000_000);

    assert_eq!(
        read_ram_u8(&machine, RESULT) & 0x80,
        0x80,
        "receive without DSR reports the timeout bit"
    );
    assert_eq!(
        read_ram_u8(&machine, RESULT + 1) & 0x80,
        0x80,
        "send without CTS/DSR reports the timeout bit"
    );
}

#[test]
fn port_without_bda_base_returns_untouched() {
    let mut machine = create_machine_dx50();
    boot_and_run(&mut machine, INVALID_PORT_CODE, &[], 1_000_000);

    assert_eq!(
        read_ram_u16(&machine, RESULT),
        0x02A5,
        "AX preserved for a port without a BDA base"
    );
    assert_eq!(
        read_ram_u16(&machine, RESULT + 2) & FLAGS_CARRY,
        0,
        "carry untouched"
    );
}
