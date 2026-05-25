use common::{CpuMode, DosBootStage, MachineModel, Tracing};

static FONT_ROM_DATA: &[u8] = include_bytes!("../../../utils/font/font.rom");

#[derive(Default)]
struct RecordingTracer {
    boot_stages: Vec<DosBootStage>,
    os_dispatch_vectors: Vec<u8>,
}

impl Tracing for RecordingTracer {
    fn trace_dos_boot(
        &mut self,
        stage: DosBootStage,
        _cpu: &dyn common::CpuAccess,
        _memory: &dyn common::MemoryAccess,
    ) {
        self.boot_stages.push(stage);
    }

    fn trace_dos_dispatch(
        &mut self,
        vector: u8,
        _cpu: &dyn common::CpuAccess,
        _memory: &dyn common::MemoryAccess,
    ) {
        self.os_dispatch_vectors.push(vector);
    }
}

#[test]
fn machine_owned_tracer_receives_hle_os_trace_callbacks() {
    let mut machine = machine::Machine::<cpu::I386, RecordingTracer>::new(
        cpu::I386::new(),
        machine::Pc9801Bus::<RecordingTracer>::new(MachineModel::PC9801RA, CpuMode::High, 48_000),
    );
    machine.bus.load_font_rom(FONT_ROM_DATA);
    machine.bus.set_xms_32_enabled(true);

    let mut total_cycles = 0u64;
    while total_cycles < 500_000_000 {
        total_cycles += machine.run_for(1_000_000);
        let tracer = machine.bus.tracer();
        let saw_boot_end = tracer.boot_stages.contains(&DosBootStage::End);
        let saw_os_dispatch = tracer.os_dispatch_vectors.contains(&0x21);
        if saw_boot_end && saw_os_dispatch {
            return;
        }
    }

    let tracer = machine.bus.tracer();
    panic!(
        "did not observe expected OS trace callbacks within budget; boot_stages={:?} os_dispatch_vectors={:?}",
        tracer.boot_stages, tracer.os_dispatch_vectors
    );
}
