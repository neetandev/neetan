use common::{OwnedTraceEvent, TraceContext, TraceEvent, TraceSink};
use machine_msx::{MsxBus, MsxMachine, MsxModel};

#[derive(Default)]
struct CanonicalTrace {
    bytes: Vec<u8>,
}

impl TraceSink for CanonicalTrace {
    fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
        let event = OwnedTraceEvent::from(event);
        self.bytes
            .extend_from_slice(format!("{}:{event:?}\n", context.tick).as_bytes());
    }
}

fn signature_machine<T: TraceSink>(tracer: T) -> MsxMachine<T> {
    let mut bus = MsxBus::new_with_trace_sink(MsxModel::Msx, 48_000, tracer);
    bus.load_synthetic_program(&[
        0x3E, 0x42, // LD A,0x42
        0x32, 0x00, 0x40, // LD (0x4000),A
        0xDB, 0x98, // IN A,(0x98)
        0xC3, 0x00, 0x00, // JP 0
    ])
    .unwrap();
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    MsxMachine::new(main_cpu, bus)
}

#[test]
fn synthetic_program_and_scheduler_are_deterministic() {
    let mut first = signature_machine(common::NoTrace);
    let mut second = signature_machine(common::NoTrace);

    for budget in [37, 191, 503, 2_000] {
        assert_eq!(first.run_for(budget), second.run_for(budget));
    }

    assert_eq!(
        first.main_cpu.capture_state(),
        second.main_cpu.capture_state()
    );
    assert_eq!(first.bus.peek_byte(0x4000), 0x42);
    assert_eq!(first.bus.peek_byte(0x4000), second.bus.peek_byte(0x4000));
    assert_eq!(first.bus.current_cycle(), second.bus.current_cycle());
    assert_eq!(first.bus.scanline(), second.bus.scanline());
    assert_eq!(
        first.bus.completed_scanlines(),
        second.bus.completed_scanlines()
    );
}

#[test]
fn canonical_trace_is_byte_for_byte_stable() {
    let mut first = signature_machine(CanonicalTrace::default());
    let mut second = signature_machine(CanonicalTrace::default());
    first.run_for(500);
    second.run_for(500);

    assert_eq!(first.bus.tracer().bytes, second.bus.tracer().bytes);
    let trace = String::from_utf8(first.bus.tracer().bytes.clone()).unwrap();
    assert!(trace.contains("Fetch"));
    assert!(trace.contains("Write"));
    assert!(trace.contains("cpu.main.io"));
    assert!(trace.contains("msx.video.scanline"));
}
