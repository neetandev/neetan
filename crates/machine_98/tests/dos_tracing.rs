use common::{BUILTIN_FONT_ROM, CpuMode, MachineModel, TraceSink};

#[derive(Default)]
struct RecordingTracer {
    calls: Vec<common::OwnedTraceCall>,
}

impl TraceSink for RecordingTracer {
    fn trace(&mut self, _context: common::TraceContext, event: common::TraceEvent<'_>) {
        if let common::TraceEvent::Call(call) = event {
            let common::OwnedTraceEvent::Call(call) =
                common::OwnedTraceEvent::from(common::TraceEvent::Call(call))
            else {
                unreachable!();
            };
            self.calls.push(call);
        }
    }
}

#[test]
fn machine_owned_tracer_receives_hle_os_trace_callbacks() {
    let mut machine = machine_98::Pc98Machine::<cpu::I386, RecordingTracer>::new(
        cpu::I386::new(),
        machine_98::Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801RA,
            CpuMode::High,
            48_000,
            RecordingTracer::default(),
        ),
    );
    machine.bus.load_font_rom(BUILTIN_FONT_ROM);
    machine.bus.set_xms_32_enabled(true);

    let mut total_cycles = 0u64;
    while total_cycles < 500_000_000 {
        total_cycles += machine.run_for(1_000_000);
        let tracer = machine.bus.tracer();
        let saw_boot_end = tracer.calls.iter().any(|call| {
            call.provider == "neetan.dos"
                && call.interface == common::TraceCallInterface::Named("boot")
                && call.phase == common::TraceCallPhase::Exit
        });
        let saw_os_dispatch = tracer.calls.iter().any(|call| {
            call.provider == "neetan.dos"
                && call.interface == common::TraceCallInterface::Interrupt(0x21)
                && call.phase == common::TraceCallPhase::Enter
        });
        if saw_boot_end && saw_os_dispatch {
            return;
        }
    }

    let tracer = machine.bus.tracer();
    panic!(
        "did not observe expected DOS call boundaries within budget; calls={:?}",
        tracer.calls
    );
}
