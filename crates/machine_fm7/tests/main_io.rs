//! Main `0xFDxx` I/O tests.

mod harness;

use common::{Bus, TraceAccess, TraceContext, TraceSink};
use harness::synthetic_roms;
use machine_fm7::{BootMode, Fm7Bus, Fm7Model, MainBusView};

#[derive(Default)]
struct CountingTracer {
    unhandled_reads: u32,
    unhandled_writes: u32,
    last_unhandled_read: u16,
    last_unhandled_write: u16,
    last_unhandled_write_value: u8,
    accesses: Vec<TraceAccess>,
    contexts: Vec<TraceContext>,
}

impl TraceSink for CountingTracer {
    fn trace(&mut self, context: TraceContext, event: common::TraceEvent<'_>) {
        if let common::TraceEvent::Access(access) = event {
            self.accesses.push(access);
            self.contexts.push(context);
            if access.space == common::TraceAddressSpace::MAIN_MEMORY && !access.handled {
                match access.kind {
                    common::TraceAccessKind::Read => {
                        self.unhandled_reads += 1;
                        self.last_unhandled_read = access.address as u16;
                    }
                    common::TraceAccessKind::Write => {
                        self.unhandled_writes += 1;
                        self.last_unhandled_write = access.address as u16;
                        self.last_unhandled_write_value = access.value.unwrap_or_default() as u8;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[test]
fn base_ports_report_their_idle_state() {
    let mut bus = Fm7Bus::new_with_trace_sink(
        Fm7Model::Fm7,
        BootMode::Basic,
        48_000,
        CountingTracer::default(),
    );
    let roms = synthetic_roms(Fm7Model::Fm7);
    bus.load_roms(&roms);

    assert_eq!(bus.read_byte(0xFD00).0, 0x7F);
    assert_eq!(bus.read_byte(0xFD01).0, 0x00);
    assert_eq!(bus.read_byte(0xFD02).0, 0x7F);
    // Bit 7 of 0xFD04/0xFD05 reports the sub busy flag, which is clear at idle.
    // 0xFD05 bit 0 is active-low external/FDC detect, so it is clear when fitted.
    assert_eq!(bus.read_byte(0xFD04).0, 0x7F);
    assert_eq!(bus.read_byte(0xFD05).0, 0x7E);

    bus.write_byte(0xFD00, 0xC3);
    bus.write_byte(0xFD01, 0x5A);
    bus.write_byte(0xFD03, 0xC0);
    bus.write_byte(0xFD04, 0xFF);
    bus.write_byte(0xFD05, 0xC0);
}

#[test]
fn unhandled_fd_ports_trace_and_return_open_bus() {
    let mut bus = Fm7Bus::new_with_trace_sink(
        Fm7Model::Fm7,
        BootMode::Basic,
        48_000,
        CountingTracer::default(),
    );
    let roms = synthetic_roms(Fm7Model::Fm7);
    bus.load_roms(&roms);

    {
        let mut view = MainBusView { bus: &mut bus };
        assert_eq!(Bus::read_byte(&mut view, 0xFD80), 0xFF);
        Bus::write_byte(&mut view, 0xFD81, 0x66);
    }

    let tracer = bus.tracer();
    assert_eq!(tracer.accesses.len(), 2);
    assert!(
        tracer
            .accesses
            .iter()
            .all(|access| access.space == common::TraceAddressSpace::MAIN_MEMORY)
    );
    assert_eq!(tracer.unhandled_reads, 1);
    assert_eq!(tracer.last_unhandled_read, 0xFD80);
    assert_eq!(tracer.accesses[0].value, Some(0xFF));
    assert_eq!(tracer.unhandled_writes, 1);
    assert_eq!(tracer.last_unhandled_write, 0xFD81);
    assert_eq!(tracer.last_unhandled_write_value, 0x66);
    assert_eq!(tracer.contexts[0].source, common::trace_source::CPU_MAIN);
    assert_eq!(
        tracer.contexts[0].clock_rate,
        Some(common::TraceRate::from_hz(u64::from(bus.cpu_clock_hz())))
    );
}
