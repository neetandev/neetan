//! SCC glue: the Sharp mouse on channel B and the RS-232C stub on channel A.

use common::TraceSink;
use device::z8530::{SccChannel, SccWriteEffect};

use super::X68kBus;
use crate::clock::cycle_to_tick;

/// Z8530 PCLK frequency in Hz.
pub(super) const SCC_CLOCK_HZ: u64 = 5_000_000;

/// Offset of the channel B control port within the SCC window.
const SCC_B_CONTROL_OFFSET: u32 = 1;
/// Offset of the channel B data port within the SCC window.
const SCC_B_DATA_OFFSET: u32 = 3;
/// Offset of the channel A control port within the SCC window.
const SCC_A_CONTROL_OFFSET: u32 = 5;
/// Offset of the channel A data port within the SCC window.
const SCC_A_DATA_OFFSET: u32 = 7;

/// Mouse status bit reporting the left button held.
const MOUSE_STATUS_LEFT: u8 = 0x01;
/// Mouse status bit reporting the right button held.
const MOUSE_STATUS_RIGHT: u8 = 0x02;
/// Mouse status bit reporting a positive X delta overflow.
const MOUSE_STATUS_X_OVERFLOW: u8 = 0x10;
/// Mouse status bit reporting a negative X delta underflow.
const MOUSE_STATUS_X_UNDERFLOW: u8 = 0x20;
/// Mouse status bit reporting a positive Y delta overflow.
const MOUSE_STATUS_Y_OVERFLOW: u8 = 0x40;
/// Mouse status bit reporting a negative Y delta underflow.
const MOUSE_STATUS_Y_UNDERFLOW: u8 = 0x80;

/// Host mouse input accumulated between MSCTRL polls.
#[derive(Debug, Default)]
pub(super) struct MouseState {
    /// Accumulated X movement since the last packet.
    delta_x: i32,
    /// Accumulated Y movement since the last packet.
    delta_y: i32,
    /// Left button held.
    left: bool,
    /// Right button held.
    right: bool,
}

/// Clamps an accumulated delta into a packet byte plus its overflow bits.
fn clamp_delta(delta: i32, overflow_bit: u8, underflow_bit: u8) -> (u8, u8) {
    if delta > 127 {
        (127, overflow_bit)
    } else if delta < -128 {
        (0x80, underflow_bit)
    } else {
        (delta as u8, 0)
    }
}

impl<T: TraceSink> X68kBus<T> {
    /// Accumulates host mouse movement for the next packet.
    pub fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.mouse.delta_x = self.mouse.delta_x.saturating_add(i32::from(delta_x));
        self.mouse.delta_y = self.mouse.delta_y.saturating_add(i32::from(delta_y));
    }

    /// Updates the held mouse buttons.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.mouse.left = left;
        self.mouse.right = right;
    }

    /// Reads an SCC register byte at an odd address.
    pub(super) fn read_scc_register(&mut self, address: u32) -> u8 {
        match address & 7 {
            SCC_B_CONTROL_OFFSET => self.scc.read_control(SccChannel::B),
            SCC_B_DATA_OFFSET => self.scc.read_data(SccChannel::B),
            SCC_A_CONTROL_OFFSET => self.scc.read_control(SccChannel::A),
            SCC_A_DATA_OFFSET => self.scc.read_data(SccChannel::A),
            other => unreachable!("even SCC offset {other} is bus-error checked"),
        }
    }

    /// Writes an SCC register byte at an odd address.
    pub(super) fn write_scc_register(&mut self, address: u32, value: u8) {
        match address & 7 {
            SCC_B_CONTROL_OFFSET => {
                if self.scc.write_control(SccChannel::B, value) == SccWriteEffect::MouseRequestEdge
                {
                    self.latch_mouse_packet();
                }
            }
            SCC_B_DATA_OFFSET => self.scc.write_data(SccChannel::B, value),
            SCC_A_CONTROL_OFFSET => {
                if self.scc.write_control(SccChannel::A, value) == SccWriteEffect::MouseRequestEdge
                {
                    self.latch_mouse_packet();
                }
            }
            SCC_A_DATA_OFFSET => self.scc.write_data(SccChannel::A, value),
            other => unreachable!("even SCC offset {other} is bus-error checked"),
        }
    }

    /// Builds a mouse packet from the accumulated input and latches it.
    fn latch_mouse_packet(&mut self) {
        let (delta_x, x_status) = clamp_delta(
            self.mouse.delta_x,
            MOUSE_STATUS_X_OVERFLOW,
            MOUSE_STATUS_X_UNDERFLOW,
        );
        let (delta_y, y_status) = clamp_delta(
            self.mouse.delta_y,
            MOUSE_STATUS_Y_OVERFLOW,
            MOUSE_STATUS_Y_UNDERFLOW,
        );
        let mut status = x_status | y_status;
        if self.mouse.left {
            status |= MOUSE_STATUS_LEFT;
        }
        if self.mouse.right {
            status |= MOUSE_STATUS_RIGHT;
        }
        self.mouse.delta_x = 0;
        self.mouse.delta_y = 0;
        let tick = cycle_to_tick(self.current_cycle, SCC_CLOCK_HZ, self.cpu_clock_hz);
        self.scc.load_mouse_packet([status, delta_x, delta_y], tick);
    }
}

#[cfg(test)]
mod tests {
    use common::{Bus, M68000AccessSize, M68000FunctionCode};

    use crate::{
        X68kBus, X68kModel,
        bus::test_support::{access, bus},
    };

    fn write_register(bus: &mut X68kBus, address: u32, value: u8) {
        bus.m68000_write(
            access(
                address,
                M68000AccessSize::Byte,
                M68000FunctionCode::SupervisorData,
            ),
            u16::from(value),
        )
        .unwrap();
    }

    fn read_register(bus: &mut X68kBus, address: u32) -> u8 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap() as u8
    }

    /// Programs the mouse driver setup on channel B through the bus.
    fn program_mouse(bus: &mut X68kBus) {
        write_register(bus, 0xE98001, 9);
        write_register(bus, 0xE98001, 0x09);
        write_register(bus, 0xE98001, 2);
        write_register(bus, 0xE98001, 0x40);
        write_register(bus, 0xE98001, 1);
        write_register(bus, 0xE98001, 0x10);
        write_register(bus, 0xE98001, 5);
        write_register(bus, 0xE98001, 0x60);
    }

    /// Pulses MSCTRL by raising RTS on channel B.
    fn pulse_msctrl(bus: &mut X68kBus) {
        write_register(bus, 0xE98001, 5);
        write_register(bus, 0xE98001, 0x62);
        write_register(bus, 0xE98001, 5);
        write_register(bus, 0xE98001, 0x60);
    }

    /// One mouse byte time in CPU cycles: 11 bits at 4800 bps from the 5 MHz
    /// PCLK, doubled into the 10 MHz CPU clock.
    const MOUSE_BYTE_CYCLES: u64 = 23_232;

    /// Fires scheduled events until the SCC raises its level 5 interrupt.
    fn wait_for_mouse_byte(bus: &mut X68kBus) {
        for _ in 0..64 {
            if bus.m68000_interrupt_level() == 5 {
                return;
            }
            let deadline = bus.next_event_cycle().expect("a byte release is due");
            bus.set_current_cycle(deadline);
            bus.process_due_events();
        }
        panic!("mouse byte never raised the SCC interrupt");
    }

    #[test]
    fn mouse_packet_arrives_through_level_5_vectored_interrupts() {
        let mut bus = bus(X68kModel::X68000);
        program_mouse(&mut bus);
        assert_eq!(bus.m68000_interrupt_level(), 0);

        bus.push_mouse_delta(5, -3);
        bus.push_mouse_delta(2, -2);
        bus.set_mouse_buttons(true, false);
        pulse_msctrl(&mut bus);
        // The packet is latched but its first byte is still on the wire.
        assert_eq!(bus.m68000_interrupt_level(), 0);

        let mut packet = [0u8; 3];
        for byte in &mut packet {
            wait_for_mouse_byte(&mut bus);
            assert_eq!(bus.m68000_acknowledge_interrupt(5), 0x44);
            *byte = read_register(&mut bus, 0xE98003);
            write_register(&mut bus, 0xE98001, 0x38);
        }
        assert_eq!(packet, [0x01, 7, 0xFB]);
        assert_eq!(bus.m68000_interrupt_level(), 0);

        // The accumulators were consumed by the packet latch.
        pulse_msctrl(&mut bus);
        wait_for_mouse_byte(&mut bus);
        assert_eq!(bus.m68000_acknowledge_interrupt(5), 0x44);
        assert_eq!(read_register(&mut bus, 0xE98003), 0x01);
        write_register(&mut bus, 0xE98001, 0x38);
        wait_for_mouse_byte(&mut bus);
        assert_eq!(bus.m68000_acknowledge_interrupt(5), 0x44);
        assert_eq!(read_register(&mut bus, 0xE98003), 0);
    }

    #[test]
    fn packet_bytes_pace_at_the_mouse_serial_rate() {
        let mut bus = bus(X68kModel::X68000);
        program_mouse(&mut bus);
        bus.push_mouse_delta(1, 0);
        pulse_msctrl(&mut bus);

        // The first byte lands one serial byte time after the MSCTRL latch.
        bus.set_current_cycle(MOUSE_BYTE_CYCLES - 1);
        bus.process_due_events();
        assert_eq!(bus.m68000_interrupt_level(), 0);
        bus.set_current_cycle(MOUSE_BYTE_CYCLES);
        bus.process_due_events();
        assert_eq!(bus.m68000_interrupt_level(), 5);
        assert_eq!(bus.m68000_acknowledge_interrupt(5), 0x44);
        read_register(&mut bus, 0xE98003);
        write_register(&mut bus, 0xE98001, 0x38);

        // The second byte keeps the cadence.
        assert_eq!(bus.m68000_interrupt_level(), 0);
        bus.set_current_cycle(2 * MOUSE_BYTE_CYCLES);
        bus.process_due_events();
        assert_eq!(bus.m68000_interrupt_level(), 5);
    }

    #[test]
    fn large_deltas_clamp_and_set_the_overflow_bits() {
        let mut bus = bus(X68kModel::X68000);
        program_mouse(&mut bus);

        bus.push_mouse_delta(300, -300);
        pulse_msctrl(&mut bus);
        wait_for_mouse_byte(&mut bus);
        bus.m68000_acknowledge_interrupt(5);
        let status = read_register(&mut bus, 0xE98003);
        write_register(&mut bus, 0xE98001, 0x38);
        wait_for_mouse_byte(&mut bus);
        bus.m68000_acknowledge_interrupt(5);
        let delta_x = read_register(&mut bus, 0xE98003);
        write_register(&mut bus, 0xE98001, 0x38);
        wait_for_mouse_byte(&mut bus);
        bus.m68000_acknowledge_interrupt(5);
        let delta_y = read_register(&mut bus, 0xE98003);
        write_register(&mut bus, 0xE98001, 0x38);

        assert_eq!(status, 0x10 | 0x80);
        assert_eq!(delta_x, 127);
        assert_eq!(delta_y, 0x80);
    }

    #[test]
    fn channel_a_stub_reports_an_empty_transmitter() {
        let mut bus = bus(X68kModel::X68000);
        // RR0 on channel A: transmit buffer empty, nothing received.
        assert_eq!(read_register(&mut bus, 0xE98005), 0x04);
        assert_eq!(read_register(&mut bus, 0xE98007), 0);
        // Baud rate readback through RR12.
        write_register(&mut bus, 0xE98005, 12);
        assert_eq!(read_register(&mut bus, 0xE98005), 14);
    }

    #[test]
    fn even_scc_addresses_raise_bus_errors() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0xE98000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_write(access(0xE98002, M68000AccessSize::Byte, supervisor), 0)
                .is_err()
        );
        // Word reads reach the odd byte with an open high byte.
        assert_eq!(
            bus.m68000_read(access(0xE98004, M68000AccessSize::Word, supervisor)),
            Ok(0xFF04)
        );
    }
}
