//! Integration tests for the FM Towns RS-MIDI path.
//!
//! FM Towns games such as Genocide emit MIDI as a raw byte stream through the
//! built-in RS-232C USART (data register at I/O 0x0A00). These tests drive the
//! real I/O dispatch and RS-232C wiring through the public machine API and check
//! that transmitted bytes are captured verbatim.

#[path = "common/harness.rs"]
mod harness;

use common::Bus;
use harness::machine_mx;
use machine_towns::TownsMachine;

const RS232C_DATA_PORT: u16 = 0x0A00;
const RS232C_STATUS_PORT: u16 = 0x0A02;
const STATUS_TXRDY: u8 = 1 << 0;
const STATUS_TXEMPTY: u8 = 1 << 2;

fn towns() -> TownsMachine<{ cpu::CPU_MODEL_486_DX }> {
    machine_mx()
}

#[test]
fn rs_midi_bytes_captured_verbatim_and_in_order() {
    let mut machine = towns();
    machine.bus.enable_midi_capture();

    // Note on, note off, program change.
    let stream = [0x90, 0x40, 0x7F, 0x80, 0x40, 0x00, 0xC0, 0x30];
    for &byte in &stream {
        machine.bus.io_write_byte(RS232C_DATA_PORT, byte);
    }

    let mut drained = Vec::new();
    machine.bus.flush_midi_into(&mut drained);
    assert_eq!(drained, stream);

    // Draining again yields nothing.
    let mut again = Vec::new();
    machine.bus.flush_midi_into(&mut again);
    assert!(again.is_empty());
}

#[test]
fn rs232c_transmitter_always_ready() {
    let mut machine = towns();
    let status = machine.bus.io_read_byte(RS232C_STATUS_PORT);
    assert_ne!(status & STATUS_TXRDY, 0, "TxRDY must be set");
    assert_ne!(status & STATUS_TXEMPTY, 0, "TxEMPTY must be set");
}

#[test]
fn midi_capture_disabled_by_default() {
    let mut machine = towns();
    for byte in [0x90, 0x40, 0x7F] {
        machine.bus.io_write_byte(RS232C_DATA_PORT, byte);
    }
    let mut drained = Vec::new();
    machine.bus.flush_midi_into(&mut drained);
    assert!(drained.is_empty());
}
