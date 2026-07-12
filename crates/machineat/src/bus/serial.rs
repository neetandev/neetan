//! COM1 serial glue: the 16450 UART (0x3F8-0x3FF, IRQ 4) and the Microsoft
//! serial mouse attached to it.
//!
//! The UART is protocol-agnostic. The mouse lives here: it accumulates host
//! movement and button state, encodes the Microsoft 3-byte protocol, and hands
//! the bytes to the UART, which paces them out at the programmed baud rate on
//! the `UartRx` event. A rising edge on DTR or RTS is a power-on reset, to which
//! the mouse answers with its `M` identification byte.

use common::Tracing;
use device::ins8250_uart::UartWriteEffect;

use crate::{
    bus::{AtBus, IRQ_COM1},
    scheduler::EventAt,
};

/// Microsoft serial mouse identification byte sent after a power-on reset.
const MOUSE_IDENTIFICATION: u8 = b'M';

/// Microsoft serial mouse state accumulated from the host.
#[derive(Debug, Default)]
pub(crate) struct SerialMouse {
    /// Accumulated horizontal movement not yet reported.
    delta_x: i32,
    /// Accumulated vertical movement not yet reported.
    delta_y: i32,
    /// Left button held.
    left: bool,
    /// Right button held.
    right: bool,
    /// Movement or button change awaiting a report.
    dirty: bool,
}

impl SerialMouse {
    /// Builds a mouse with no pending movement.
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

/// Encodes a Microsoft 3-byte mouse packet.
fn encode_ms_packet(delta_x: i32, delta_y: i32, left: bool, right: bool) -> [u8; 3] {
    let delta_x = delta_x.clamp(-128, 127);
    let delta_y = delta_y.clamp(-128, 127);
    let byte0 = 0x40
        | if left { 0x20 } else { 0 }
        | if right { 0x10 } else { 0 }
        | (((delta_y >> 6) & 0x03) as u8) << 2
        | ((delta_x >> 6) & 0x03) as u8;
    let byte1 = (delta_x & 0x3F) as u8;
    let byte2 = (delta_y & 0x3F) as u8;
    [byte0, byte1, byte2]
}

impl<T: Tracing> AtBus<T> {
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
                .queue_received_bytes(&[MOUSE_IDENTIFICATION], self.current_cycle);
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
        if delta_x != 0 || delta_y != 0 {
            self.serial_mouse.delta_x =
                self.serial_mouse.delta_x.saturating_add(i32::from(delta_x));
            self.serial_mouse.delta_y =
                self.serial_mouse.delta_y.saturating_add(i32::from(delta_y));
            self.serial_mouse.dirty = true;
        }
        self.emit_mouse_packet();
    }

    /// Updates host mouse button state and streams a report on change.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        if left != self.serial_mouse.left || right != self.serial_mouse.right {
            self.serial_mouse.left = left;
            self.serial_mouse.right = right;
            self.serial_mouse.dirty = true;
        }
        self.emit_mouse_packet();
    }

    /// Sends one mouse packet if a report is pending and the UART is idle.
    fn emit_mouse_packet(&mut self) {
        if !self.serial_mouse.dirty || self.uart_com1.next_event_cycle().is_some() {
            return;
        }
        let packet = encode_ms_packet(
            self.serial_mouse.delta_x,
            self.serial_mouse.delta_y,
            self.serial_mouse.left,
            self.serial_mouse.right,
        );
        self.serial_mouse.delta_x = 0;
        self.serial_mouse.delta_y = 0;
        self.serial_mouse.dirty = false;
        self.uart_com1
            .queue_received_bytes(&packet, self.current_cycle);
        self.reschedule_uart_rx();
        self.sync_com1_irq();
    }
}

#[cfg(test)]
mod tests {
    use super::encode_ms_packet;

    #[test]
    fn encodes_movement_and_buttons() {
        // Left held, right released, dx = +5, dy = -3.
        assert_eq!(encode_ms_packet(5, -3, true, false), [0x6C, 0x05, 0x3D]);
    }

    #[test]
    fn encodes_neutral_packet() {
        assert_eq!(encode_ms_packet(0, 0, false, false), [0x40, 0x00, 0x00]);
    }

    #[test]
    fn clamps_large_deltas_and_packs_high_bits() {
        let [byte0, byte1, byte2] = encode_ms_packet(300, -300, false, false);
        // dx clamps to 127 (high bits 01), dy clamps to -128 (high bits 10).
        assert_eq!(byte0 & 0x03, 0x01);
        assert_eq!((byte0 >> 2) & 0x03, 0x02);
        assert_eq!(byte1, 127 & 0x3F);
        assert_eq!(byte2, (-128i32 & 0x3F) as u8);
    }
}
