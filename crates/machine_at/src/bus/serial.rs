//! COM1 serial glue: the 16450 UART (0x3F8-0x3FF, IRQ 4) and the Microsoft
//! serial mouse attached to it.
//!
//! The UART is protocol-agnostic. The mouse lives here: it accumulates host
//! movement and button state, encodes the Microsoft 3-byte protocol, and hands
//! the bytes to the UART, which paces them out at the programmed baud rate on
//! the `UartRx` event. A rising edge on DTR or RTS is a power-on reset, to which
//! the mouse answers with its `M` identification byte.

use common::TraceSink;
use device::{ins8250_uart::UartWriteEffect, mouse_serial::SERIAL_MOUSE_IDENTIFICATION};

use crate::{
    bus::{AtBus, IRQ_COM1},
    scheduler::EventAt,
};

impl<T: TraceSink> AtBus<T> {
    /// Reads a COM1 UART register (ports 0x3F8-0x3FF).
    pub(super) fn serial_io_read(&mut self, port: u16) -> u8 {
        let register = (port - 0x03F8) as u8;
        let value = self.uart_com1.read(register);
        self.sync_com1_irq();
        value
    }

    /// Writes a COM1 UART register, handling the mouse power-on reset.
    pub(super) fn serial_io_write(&mut self, port: u16, value: u8) {
        let register = (port - 0x03F8) as u8;
        let effect = self.uart_com1.write(register, value);
        if let UartWriteEffect::ModemControlChanged {
            dtr_rose, rts_rose, ..
        } = effect
            && (dtr_rose || rts_rose)
        {
            self.uart_com1
                .queue_received_bytes(&[SERIAL_MOUSE_IDENTIFICATION], self.current_cycle);
            self.reschedule_uart_rx();
        }
        self.sync_com1_irq();
    }

    /// Raises or clears IRQ 4 from the UART interrupt output.
    pub(crate) fn sync_com1_irq(&mut self) {
        if self.uart_com1.irq_asserted() {
            self.raise_irq(IRQ_COM1);
        } else {
            self.clear_irq(IRQ_COM1);
        }
    }

    /// (Re)schedules or cancels the paced UART receive event.
    pub(crate) fn reschedule_uart_rx(&mut self) {
        match self.uart_com1.next_event_cycle() {
            Some(cycle) => self.scheduler.schedule(EventAt::UartRx, cycle),
            None => self.scheduler.cancel(EventAt::UartRx),
        }
        self.update_next_event_cycle();
    }

    /// Accumulates host mouse movement and streams a report when idle.
    pub fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.serial_mouse.push_delta(delta_x, delta_y);
        self.emit_mouse_packet();
    }

    /// Updates host mouse button state and streams a report on change.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.serial_mouse.set_buttons(left, right);
        self.emit_mouse_packet();
    }

    /// Sends one mouse packet if a report is pending and the UART is idle.
    fn emit_mouse_packet(&mut self) {
        if self.uart_com1.next_event_cycle().is_some() {
            return;
        }
        let Some(packet) = self.serial_mouse.take_packet() else {
            return;
        };
        self.uart_com1
            .queue_received_bytes(&packet, self.current_cycle);
        self.reschedule_uart_rx();
        self.sync_com1_irq();
    }
}
