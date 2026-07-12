//! National Semiconductor INS8250 / 16450 UART for the PC/AT COM ports.
//!
//! Behavioral model of the 16450 (a 16550 without the FIFO). It carries the
//! serial mouse: the machine glue latches received bytes with
//! [`Ins8250Uart::queue_received_bytes`], the UART paces them out one serial
//! frame apart at the programmed baud rate, and raises the received-data
//! interrupt per byte. Loopback (MCR bit 4) is modeled so the BIOS and mouse
//! driver self tests pass. Transmit is instantaneous and discarded; the
//! transmitter holding register always reads empty.

use std::collections::VecDeque;

/// Interrupt enable: received-data-available.
const IER_ERBFI: u8 = 1 << 0;
/// Interrupt enable: transmitter-holding-register-empty.
const IER_ETBEI: u8 = 1 << 1;
/// Interrupt enable: receiver-line-status.
const IER_ELSI: u8 = 1 << 2;
/// Interrupt enable: modem-status.
const IER_EDSSI: u8 = 1 << 3;

/// IIR value: no interrupt pending (bit 0 set).
const IIR_NONE: u8 = 0x01;
/// IIR value: modem status (lowest priority).
const IIR_MODEM_STATUS: u8 = 0x00;
/// IIR value: transmitter holding register empty.
const IIR_THR_EMPTY: u8 = 0x02;
/// IIR value: received data available.
const IIR_RX_DATA: u8 = 0x04;
/// IIR value: receiver line status (highest priority).
const IIR_LINE_STATUS: u8 = 0x06;

/// Line control: divisor latch access bit.
const LCR_DLAB: u8 = 1 << 7;

/// Modem control: data terminal ready.
const MCR_DTR: u8 = 1 << 0;
/// Modem control: request to send.
const MCR_RTS: u8 = 1 << 1;
/// Modem control: auxiliary output 1.
const MCR_OUT1: u8 = 1 << 2;
/// Modem control: auxiliary output 2 (gates the interrupt onto the bus).
const MCR_OUT2: u8 = 1 << 3;
/// Modem control: loopback test mode.
const MCR_LOOP: u8 = 1 << 4;

/// Line status: data ready.
const LSR_DR: u8 = 1 << 0;
/// Line status: overrun error.
const LSR_OE: u8 = 1 << 1;
/// Line status: parity error.
const LSR_PE: u8 = 1 << 2;
/// Line status: framing error.
const LSR_FE: u8 = 1 << 3;
/// Line status: break interrupt.
const LSR_BI: u8 = 1 << 4;
/// Line status: transmitter holding register empty (always set here).
const LSR_THRE: u8 = 1 << 5;
/// Line status: transmitter empty (always set here).
const LSR_TEMT: u8 = 1 << 6;
/// Line status error bits, cleared by a status read.
const LSR_ERROR: u8 = LSR_OE | LSR_PE | LSR_FE | LSR_BI;

/// Modem status delta: clear-to-send changed.
const MSR_DELTA_CTS: u8 = 1 << 0;
/// Modem status delta: data-set-ready changed.
const MSR_DELTA_DSR: u8 = 1 << 1;
/// Modem status delta: trailing edge of ring indicator.
const MSR_DELTA_TERI: u8 = 1 << 2;
/// Modem status delta: data-carrier-detect changed.
const MSR_DELTA_DCD: u8 = 1 << 3;
/// Modem status level: clear to send.
const MSR_CTS: u8 = 1 << 4;
/// Modem status level: data set ready.
const MSR_DSR: u8 = 1 << 5;
/// Modem status level: ring indicator.
const MSR_RI: u8 = 1 << 6;
/// Modem status level: data carrier detect.
const MSR_DCD: u8 = 1 << 7;

/// Baud rate base clock (1.8432 MHz / 16).
const BAUD_BASE: u32 = 115_200;

/// Side effect of a control-register write the machine glue must react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartWriteEffect {
    /// No externally visible effect.
    None,
    /// The modem control register changed. `dtr_rose`/`rts_rose` flag a
    /// low-to-high edge, which a serial mouse treats as a power-on reset.
    ModemControlChanged {
        /// New data-terminal-ready level.
        dtr: bool,
        /// New request-to-send level.
        rts: bool,
        /// Data-terminal-ready just rose.
        dtr_rose: bool,
        /// Request-to-send just rose.
        rts_rose: bool,
    },
}

/// Saveable INS8250 register state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ins8250UartState {
    /// Baud rate divisor latch (DLL/DLM).
    pub divisor: u16,
    /// Interrupt enable register.
    pub ier: u8,
    /// Line control register.
    pub lcr: u8,
    /// Modem control register.
    pub mcr: u8,
    /// Line status error bits (OE/PE/FE/BI); THRE/TEMT/DR are synthesized.
    pub lsr_errors: u8,
    /// Modem status delta bits (bits 0-3), cleared on read.
    pub msr_delta: u8,
    /// Scratch register.
    pub scratch: u8,
    /// Receiver buffer register (last received byte).
    pub rbr: u8,
    /// Whether the receiver buffer holds an unread byte.
    pub data_ready: bool,
    /// Latched transmitter-empty interrupt.
    pub thre_interrupt: bool,
}

/// INS8250 / 16450 UART.
pub struct Ins8250Uart {
    /// Embedded register state for save/restore.
    pub state: Ins8250UartState,
    /// Core clock in hertz, for converting the baud rate to cycles.
    cpu_clock_hz: u32,
    /// Bytes waiting to be released into the receiver (mouse packet queue).
    pending: VecDeque<u8>,
    /// Absolute cycle of the next pending byte release.
    rx_release_cycle: Option<u64>,
}

impl Ins8250Uart {
    /// Builds a UART in the power-on state.
    pub fn new(cpu_clock_hz: u32) -> Self {
        Self {
            state: Ins8250UartState {
                divisor: 0,
                ier: 0,
                lcr: 0,
                mcr: 0,
                lsr_errors: 0,
                msr_delta: 0,
                scratch: 0,
                rbr: 0,
                data_ready: false,
                thre_interrupt: false,
            },
            cpu_clock_hz,
            pending: VecDeque::new(),
            rx_release_cycle: None,
        }
    }

    /// Returns the UART to the power-on state.
    pub fn reset(&mut self) {
        let clock = self.cpu_clock_hz;
        *self = Self::new(clock);
    }

    /// Reads register `reg` (0..=7), applying divisor-latch selection.
    pub fn read(&mut self, reg: u8) -> u8 {
        let dlab = self.state.lcr & LCR_DLAB != 0;
        match reg {
            0 if dlab => self.state.divisor as u8,
            0 => {
                self.state.data_ready = false;
                self.state.rbr
            }
            1 if dlab => (self.state.divisor >> 8) as u8,
            1 => self.state.ier & 0x0F,
            2 => self.read_iir(),
            3 => self.state.lcr,
            4 => self.state.mcr,
            5 => self.read_lsr(),
            6 => self.read_msr(),
            7 => self.state.scratch,
            _ => 0xFF,
        }
    }

    /// Writes register `reg` (0..=7), returning any modem-control side effect.
    pub fn write(&mut self, reg: u8, value: u8) -> UartWriteEffect {
        let dlab = self.state.lcr & LCR_DLAB != 0;
        match reg {
            0 if dlab => {
                self.state.divisor = (self.state.divisor & 0xFF00) | u16::from(value);
            }
            0 => self.write_thr(value),
            1 if dlab => {
                self.state.divisor = (self.state.divisor & 0x00FF) | (u16::from(value) << 8);
            }
            1 => {
                self.state.ier = value & 0x0F;
                if value & IER_ETBEI != 0 {
                    self.state.thre_interrupt = true;
                }
            }
            2 => {}
            3 => self.state.lcr = value,
            4 => return self.write_mcr(value),
            5 | 6 => {}
            7 => self.state.scratch = value,
            _ => {}
        }
        UartWriteEffect::None
    }

    /// Reports whether the interrupt output is asserted (gated by MCR OUT2).
    pub fn irq_asserted(&self) -> bool {
        self.state.mcr & MCR_OUT2 != 0 && self.iir() & IIR_NONE == 0
    }

    /// Queues received bytes, scheduling their paced release from `now`.
    pub fn queue_received_bytes(&mut self, bytes: &[u8], now: u64) {
        if bytes.is_empty() {
            return;
        }
        for &byte in bytes {
            self.pending.push_back(byte);
        }
        if self.rx_release_cycle.is_none() {
            self.rx_release_cycle = Some(now.saturating_add(self.byte_duration_cycles()));
        }
    }

    /// Serial transmission time of one frame in core cycles.
    pub fn byte_duration_cycles(&self) -> u64 {
        let baud = self.baud().max(1);
        (u64::from(self.cpu_clock_hz) * self.frame_bits() / u64::from(baud)).max(1)
    }

    /// Releases every pending byte whose frame finished by `now`.
    pub fn advance_to(&mut self, now: u64) {
        while let Some(due) = self.rx_release_cycle {
            if due > now {
                break;
            }
            if let Some(byte) = self.pending.pop_front() {
                if self.state.data_ready {
                    self.state.lsr_errors |= LSR_OE;
                }
                self.state.rbr = byte;
                self.state.data_ready = true;
            }
            self.rx_release_cycle = if self.pending.is_empty() {
                None
            } else {
                Some(due.saturating_add(self.byte_duration_cycles()))
            };
        }
    }

    /// Cycle of the next pending byte release, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.rx_release_cycle
    }

    /// Programmed baud rate.
    pub fn baud(&self) -> u32 {
        BAUD_BASE / u32::from(self.state.divisor).max(1)
    }

    /// Serial frame length in bits, derived from the line control register.
    fn frame_bits(&self) -> u64 {
        let data = 5 + u64::from(self.state.lcr & 0x03);
        let parity = if self.state.lcr & 0x08 != 0 { 1 } else { 0 };
        let stop = if self.state.lcr & 0x04 != 0 { 2 } else { 1 };
        1 + data + parity + stop
    }

    /// Handles a transmitter-holding-register write.
    fn write_thr(&mut self, value: u8) {
        self.state.thre_interrupt = true;
        if self.state.mcr & MCR_LOOP != 0 {
            if self.state.data_ready {
                self.state.lsr_errors |= LSR_OE;
            }
            self.state.rbr = value;
            self.state.data_ready = true;
        }
    }

    /// Handles a modem-control-register write and its loopback modem levels.
    fn write_mcr(&mut self, value: u8) -> UartWriteEffect {
        let old = self.state.mcr;
        let old_level = self.msr_level();
        self.state.mcr = value & 0x1F;
        let new_level = self.msr_level();
        let changed = old_level ^ new_level;
        if changed & MSR_CTS != 0 {
            self.state.msr_delta |= MSR_DELTA_CTS;
        }
        if changed & MSR_DSR != 0 {
            self.state.msr_delta |= MSR_DELTA_DSR;
        }
        if old_level & MSR_RI != 0 && new_level & MSR_RI == 0 {
            self.state.msr_delta |= MSR_DELTA_TERI;
        }
        if changed & MSR_DCD != 0 {
            self.state.msr_delta |= MSR_DELTA_DCD;
        }
        let dtr = value & MCR_DTR != 0;
        let rts = value & MCR_RTS != 0;
        UartWriteEffect::ModemControlChanged {
            dtr,
            rts,
            dtr_rose: dtr && old & MCR_DTR == 0,
            rts_rose: rts && old & MCR_RTS == 0,
        }
    }

    /// Computes the modem status level bits (loopback reflects MCR outputs).
    fn msr_level(&self) -> u8 {
        if self.state.mcr & MCR_LOOP == 0 {
            return 0;
        }
        let mcr = self.state.mcr;
        (if mcr & MCR_RTS != 0 { MSR_CTS } else { 0 })
            | (if mcr & MCR_DTR != 0 { MSR_DSR } else { 0 })
            | (if mcr & MCR_OUT1 != 0 { MSR_RI } else { 0 })
            | (if mcr & MCR_OUT2 != 0 { MSR_DCD } else { 0 })
    }

    /// Identifies the highest-priority enabled and pending interrupt.
    fn iir(&self) -> u8 {
        if self.state.ier & IER_ELSI != 0 && self.state.lsr_errors & LSR_ERROR != 0 {
            IIR_LINE_STATUS
        } else if self.state.ier & IER_ERBFI != 0 && self.state.data_ready {
            IIR_RX_DATA
        } else if self.state.ier & IER_ETBEI != 0 && self.state.thre_interrupt {
            IIR_THR_EMPTY
        } else if self.state.ier & IER_EDSSI != 0 && self.state.msr_delta != 0 {
            IIR_MODEM_STATUS
        } else {
            IIR_NONE
        }
    }

    /// Reads the interrupt identification register, clearing a THRE source.
    fn read_iir(&mut self) -> u8 {
        let iir = self.iir();
        if iir == IIR_THR_EMPTY {
            self.state.thre_interrupt = false;
        }
        iir
    }

    /// Reads the line status register, clearing the error bits.
    fn read_lsr(&mut self) -> u8 {
        let value = self.state.lsr_errors
            | LSR_THRE
            | LSR_TEMT
            | if self.state.data_ready { LSR_DR } else { 0 };
        self.state.lsr_errors &= !LSR_ERROR;
        value
    }

    /// Reads the modem status register, clearing the delta bits.
    fn read_msr(&mut self) -> u8 {
        let value = self.msr_level() | (self.state.msr_delta & 0x0F);
        self.state.msr_delta = 0;
        value
    }
}
