//! Main `0xFDxx` I/O tests.

mod harness;

use common::Tracing;
use harness::synthetic_roms;
use machine_fm7::{BootMode, Fm7Bus, Fm7Model};

#[derive(Default)]
struct CountingTracer {
    unhandled_reads: u32,
    unhandled_writes: u32,
    last_unhandled_read: u16,
    last_unhandled_write: u16,
    last_unhandled_write_value: u8,
}

impl Tracing for CountingTracer {
    fn trace_io_unhandled_read(&mut self, port: u16) {
        self.unhandled_reads += 1;
        self.last_unhandled_read = port;
    }

    fn trace_io_unhandled_write(&mut self, port: u16, value: u8) {
        self.unhandled_writes += 1;
        self.last_unhandled_write = port;
        self.last_unhandled_write_value = value;
    }
}

#[test]
fn base_ports_report_their_idle_state() {
    let mut bus = Fm7Bus::<CountingTracer>::new(Fm7Model::Fm7, BootMode::Basic, 48_000);
    let roms = synthetic_roms(Fm7Model::Fm7);
    bus.load_roms(&roms);

    assert_eq!(bus.read_byte(0xFD00), 0x7F);
    assert_eq!(bus.read_byte(0xFD01), 0x00);
    assert_eq!(bus.read_byte(0xFD02), 0x7F);
    // Bit 7 of 0xFD04/0xFD05 reports the sub busy flag, which is clear at idle.
    // 0xFD05 bit 0 is active-low external/FDC detect, so it is clear when fitted.
    assert_eq!(bus.read_byte(0xFD04), 0x7F);
    assert_eq!(bus.read_byte(0xFD05), 0x7E);

    bus.write_byte(0xFD00, 0xC3);
    bus.write_byte(0xFD01, 0x5A);
    bus.write_byte(0xFD03, 0xC0);
    bus.write_byte(0xFD04, 0xFF);
    bus.write_byte(0xFD05, 0xC0);
}

#[test]
fn unhandled_fd_ports_trace_and_return_open_bus() {
    let mut bus = Fm7Bus::<CountingTracer>::new(Fm7Model::Fm7, BootMode::Basic, 48_000);
    let roms = synthetic_roms(Fm7Model::Fm7);
    bus.load_roms(&roms);

    assert_eq!(bus.read_byte(0xFD80), 0xFF);
    bus.write_byte(0xFD81, 0x66);

    let tracer = bus.tracer();
    assert_eq!(tracer.unhandled_reads, 1);
    assert_eq!(tracer.last_unhandled_read, 0xFD80);
    assert_eq!(tracer.unhandled_writes, 1);
    assert_eq!(tracer.last_unhandled_write, 0xFD81);
    assert_eq!(tracer.last_unhandled_write_value, 0x66);
}
