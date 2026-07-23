//! INT 14h serial port services.
//!
//! Single-shot polling model: send and receive test the modem and line
//! status once and report the timeout bit immediately when the UART is not
//! ready, charging the BDA timeout as guest wait cycles so polling loops
//! advance the paced UART between retries. With nothing driving the modem
//! lines the ready gates only pass in loopback mode, which matches an AT
//! with an unconnected port.

use common::{Cpu, TraceSink};

use super::AtBus;

/// BIOS data area: COM port base addresses (four words).
const BDA_COM_PORT_TABLE: u32 = 0x400;
/// BIOS data area: COM port timeout counters (four bytes).
const BDA_COM_TIMEOUT_TABLE: u32 = 0x47C;
/// Guest cycles charged per BDA timeout unit on a timed-out call.
const TIMEOUT_WAIT_CYCLES: i64 = 2000;
/// UART register offset: transmit/receive buffer (DLAB clear).
const UART_DATA: u16 = 0;
/// UART register offset: divisor latch low (DLAB set).
const UART_DIVISOR_LOW: u16 = 0;
/// UART register offset: divisor latch high (DLAB set).
const UART_DIVISOR_HIGH: u16 = 1;
/// UART register offset: line control register.
const UART_LCR: u16 = 3;
/// UART register offset: line status register.
const UART_LSR: u16 = 5;
/// UART register offset: modem status register.
const UART_MSR: u16 = 6;
/// LCR bit 7: divisor latch access.
const LCR_DLAB: u8 = 0x80;
/// LSR bit 0: received data ready.
const LSR_DATA_READY: u8 = 0x01;
/// LSR receive error bits: overrun, parity, framing, break.
const LSR_ERROR_MASK: u8 = 0x1E;
/// MSR bit 4: clear to send.
const MSR_CTS: u8 = 0x10;
/// MSR bit 5: data set ready.
const MSR_DSR: u8 = 0x20;
/// AH bit 7: the operation timed out.
const STATUS_TIMEOUT: u8 = 0x80;
/// Divisor values for the AL bits 7:5 baud rate select, 110 to 9600 baud.
const BAUD_DIVISORS: [u16; 8] = [
    0x0417, 0x0300, 0x0180, 0x00C0, 0x0060, 0x0030, 0x0018, 0x000C,
];

impl<T: TraceSink> AtBus<T> {
    /// INT 14h serial services dispatch. Returns with all registers
    /// untouched when DX names a port without a BDA base address, like the
    /// IBM BIOS early exit.
    pub(super) fn hle_int14h(&mut self, cpu: &mut impl Cpu) {
        let port_index = cpu.dx();
        if port_index > 3 {
            return;
        }
        let base = self.read_mem_word(BDA_COM_PORT_TABLE + u32::from(port_index) * 2);
        if base == 0 {
            return;
        }
        match cpu.ah() {
            0x00 => self.int14h_initialize(cpu, base),
            0x01 => self.int14h_send(cpu, base, port_index),
            0x02 => self.int14h_receive(cpu, base, port_index),
            0x03 => self.int14h_status(cpu, base),
            _ => {}
        }
    }

    /// AH=00h: programs the baud rate divisor from AL bits 7:5 and the line
    /// parameters from AL bits 4:0, then returns the port status.
    fn int14h_initialize(&mut self, cpu: &mut impl Cpu, base: u16) {
        let parameters = cpu.al();
        let divisor = BAUD_DIVISORS[usize::from(parameters >> 5)];
        let line_control = parameters & 0x1F;
        self.io_write(base + UART_LCR, LCR_DLAB | line_control);
        self.io_write(base + UART_DIVISOR_LOW, divisor as u8);
        self.io_write(base + UART_DIVISOR_HIGH, (divisor >> 8) as u8);
        self.io_write(base + UART_LCR, line_control);
        self.int14h_status(cpu, base);
    }

    /// AH=01h: sends the character in AL when the modem lines are ready,
    /// returning the line status in AH. Not ready reports the timeout bit.
    fn int14h_send(&mut self, cpu: &mut impl Cpu, base: u16, port_index: u16) {
        let modem_status = self.io_read(base + UART_MSR).0;
        if modem_status & (MSR_DSR | MSR_CTS) != (MSR_DSR | MSR_CTS) {
            self.int14h_timeout(cpu, base, port_index);
            return;
        }
        self.io_write(base + UART_DATA, cpu.al());
        cpu.set_ah(self.io_read(base + UART_LSR).0);
    }

    /// AH=02h: receives a character into AL when one is ready, returning
    /// the receive error bits in AH. Not ready reports the timeout bit.
    fn int14h_receive(&mut self, cpu: &mut impl Cpu, base: u16, port_index: u16) {
        let modem_status = self.io_read(base + UART_MSR).0;
        if modem_status & MSR_DSR == 0 {
            self.int14h_timeout(cpu, base, port_index);
            return;
        }
        let line_status = self.io_read(base + UART_LSR).0;
        if line_status & LSR_DATA_READY == 0 {
            self.int14h_timeout(cpu, base, port_index);
            return;
        }
        cpu.set_al(self.io_read(base + UART_DATA).0);
        cpu.set_ah(line_status & LSR_ERROR_MASK);
    }

    /// AH=03h: returns the line status in AH and the modem status in AL.
    fn int14h_status(&mut self, cpu: &mut impl Cpu, base: u16) {
        cpu.set_ah(self.io_read(base + UART_LSR).0);
        cpu.set_al(self.io_read(base + UART_MSR).0);
    }

    /// Timed-out call: reports the line status with the timeout bit set and
    /// charges the BDA timeout as guest wait cycles.
    fn int14h_timeout(&mut self, cpu: &mut impl Cpu, base: u16, port_index: u16) {
        let timeout = self.read_mem_byte(BDA_COM_TIMEOUT_TABLE + u32::from(port_index));
        self.pending_wait_cycles += i64::from(timeout) * TIMEOUT_WAIT_CYCLES;
        cpu.set_ah(self.io_read(base + UART_LSR).0 | STATUS_TIMEOUT);
    }
}
