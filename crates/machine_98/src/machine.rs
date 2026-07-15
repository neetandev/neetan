use common::{Bus, Cpu, unlikely};

use crate::{
    NoTrace, Pc98InspectionCpuState, Pc98InspectionState, Pc9801Bus, TraceSink, bus::Pc9801BusState,
};

save_state::runtime_state! {
/// Machine-root state for one PC-98 snapshot.
#[derive(Clone)]
struct Pc98RuntimeState<CpuState> {
    cpu: CpuState,
    bus: Pc9801BusState,
}}

/// CPU operations required by the PC-98 root save state.
pub trait Pc98RuntimeCpu: Cpu {
    /// Complete authoritative CPU state type.
    type RuntimeState: save_state::RuntimeState;

    /// Captures complete CPU state.
    fn capture_runtime_state(&self) -> Self::RuntimeState;
    /// Validates and restores complete CPU state.
    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError>;
}

impl Pc98RuntimeCpu for cpu::I8086 {
    type RuntimeState = cpu::I8086State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

impl Pc98RuntimeCpu for cpu::VX0 {
    type RuntimeState = cpu::V30State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.state.clone()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        save_state::ValidateState::validate_state(&state, &cpu::V30_BUS)?;
        self.load_state(&state);
        Ok(())
    }
}

impl Pc98RuntimeCpu for cpu::I286 {
    type RuntimeState = cpu::I286State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

impl<const CPU_MODEL: u8> Pc98RuntimeCpu for cpu::I386<CPU_MODEL> {
    type RuntimeState = cpu::I386State;

    fn capture_runtime_state(&self) -> Self::RuntimeState {
        self.capture_state()
    }

    fn restore_runtime_state(
        &mut self,
        state: Self::RuntimeState,
    ) -> Result<(), save_state::StateValidationError> {
        self.restore_state(state)
    }
}

/// Generic PC-9801 machine: a CPU wired to the shared PC-9801 bus.
pub struct Pc98Machine<C: Cpu, T: TraceSink = NoTrace> {
    /// The CPU.
    pub cpu: C,
    /// The system bus.
    pub bus: Pc9801Bus<T>,
}

impl<C: Cpu, T: TraceSink> Pc98Machine<C, T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(cpu: C, bus: Pc9801Bus<T>) -> Self {
        Self { cpu, bus }
    }

    /// Runs the machine for up to `budget` CPU cycles.
    ///
    /// When the CPU halts, advances time to the next scheduled event
    /// so that timer interrupts can fire and wake the CPU.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let slice_end = if self.cpu.halted() {
                let next_event_cycle = self.bus.next_event_cycle().unwrap_or(target_cycle);
                next_event_cycle.clamp(current_cycle + 1, target_cycle)
            } else {
                target_cycle
            };

            self.bus
                .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);
            let ran_cycles = self.cpu.run_for(slice_end - current_cycle, &mut self.bus);
            if unlikely(T::ENABLED && self.bus.tracer().yield_requested()) {
                break;
            }
            self.bus
                .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);

            if let Some(warm_ctx) = self.bus.take_reset_pending() {
                std::hint::cold_path();
                if self.bus.shutdown_requested() {
                    // System shutdown
                    break;
                } else if let Some((ss, sp, cs, ip)) = warm_ctx {
                    // Warm reset
                    self.cpu.warm_reset(ss, sp, cs, ip);
                } else {
                    // Cold reset
                    self.bus.select_rom_bank_itf();
                    self.cpu.reset();
                }
                continue;
            }

            if unlikely(self.bus.sasi_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_sasi_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.ide_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_ide_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.fdd640k_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_fdd640k_hle(
                    self.cpu.segment_base(common::SegmentRegister::SS),
                    self.cpu.sp(),
                );
                continue;
            }

            if unlikely(self.bus.bios_hle_pending()) {
                self.bus.set_hle_paging(self.cpu.cr0(), self.cpu.cr3());
                self.bus.execute_bios_hle(&mut self.cpu);
                continue;
            }

            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }

        self.bus.current_cycle() - start_cycle
    }
}

/// PC-9801F machine type (8086 CPU at 5 / 8 MHz, basic µPD7220, 20-bit address space).
pub type Pc9801F = Pc98Machine<cpu::I8086>;

/// PC-9801VM machine type (V30 CPU at 8 / 10 MHz).
pub type Pc9801Vm = Pc98Machine<cpu::VX0>;

/// PC-9801VX machine type (80286 CPU at 8 / 10 MHz).
pub type Pc9801Vx = Pc98Machine<cpu::I286>;

/// PC-9801RS machine type (80386SX CPU at 16 MHz).
pub type Pc9801Rs = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_386_SX }>>;

/// PC-9801RA machine type (80386DX CPU at 20 MHz).
pub type Pc9801Ra = Pc98Machine<cpu::I386>;

/// PC-9821AS machine type (486DX CPU at 33 MHz, IDE, PEGC).
pub type Pc9821As = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_486_DX }>>;

/// PC-9821AP machine type (486DX2 CPU at 66 MHz, IDE, PEGC).
pub type Pc9821Ap = Pc98Machine<cpu::I386<{ cpu::CPU_MODEL_486_DX }>>;

impl<T: TraceSink> Pc98Machine<cpu::I8086, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I8086(self.cpu.state.clone()))
    }
}

impl<T: TraceSink> Pc98Machine<cpu::VX0, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::V30(self.cpu.state.clone()))
    }
}

impl<T: TraceSink> Pc98Machine<cpu::I286, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I286(self.cpu.state.clone()))
    }
}

impl<const CPU_MODEL: u8, T: TraceSink> Pc98Machine<cpu::I386<CPU_MODEL>, T> {
    /// Captures the read-only compatibility inspection view.
    pub fn inspection_state(&self) -> Pc98InspectionState {
        self.bus
            .inspection_state(Pc98InspectionCpuState::I386(self.cpu.state.clone()))
    }
}

fn insert_floppy_impl<T: TraceSink>(
    bus: &mut Pc9801Bus<T>,
    drive: usize,
    path: &std::path::Path,
) -> Result<String, String> {
    let data = std::fs::read(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let image = device::floppy::load_floppy_image(path, &data)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let description = format!("{} ({})", image.name, image.format_name());
    bus.insert_floppy(drive, image, Some(path.to_path_buf()));
    Ok(description)
}

fn insert_cdrom_impl<T: TraceSink>(
    bus: &mut Pc9801Bus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let (image, description) = device::cdrom::load_cd_image(path)?;
    bus.insert_cdrom(image);
    Ok(description)
}

fn validate_hdd_for_model(
    model: common::MachineModel,
    geometry: &device::disk::HddGeometry,
) -> Result<(), String> {
    match model {
        common::MachineModel::PC9801F
        | common::MachineModel::PC9801VM
        | common::MachineModel::PC9801VX
        | common::MachineModel::PC9801RS
        | common::MachineModel::PC9801RA => {
            if geometry.sasi_media_type().is_none() {
                return Err(format!(
                    "{} is not a standard SASI geometry: {}C/{}H/{}S with {}-byte sectors",
                    model,
                    geometry.cylinders,
                    geometry.heads,
                    geometry.sectors_per_track,
                    geometry.sector_size,
                ));
            }
        }
        common::MachineModel::PC9821AS | common::MachineModel::PC9821AP => {
            if geometry.sector_size != 512 && geometry.sasi_media_type().is_none() {
                return Err(format!(
                    "{} does not support this IDE geometry: {}C/{}H/{}S with {}-byte sectors",
                    model,
                    geometry.cylinders,
                    geometry.heads,
                    geometry.sectors_per_track,
                    geometry.sector_size,
                ));
            }
        }
    }
    Ok(())
}

impl<C: Pc98RuntimeCpu, T: TraceSink> Pc98Machine<C, T> {
    fn capture_machine_blob(
        &mut self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        if !self.bus.runtime_state_supported() {
            return Err(save_state::SaveStateError::Unsupported);
        }
        let root = Pc98RuntimeState {
            cpu: self.cpu.capture_runtime_state(),
            bus: self.bus.capture_runtime_state()?,
        };
        save_state::capture_machine_state(
            root,
            self.bus.save_state_resources()?,
            self.bus.save_state_media()?,
        )
    }

    fn restore_machine_blob(
        &mut self,
        blob: &save_state::MachineStateBlob,
    ) -> Result<(), save_state::SaveStateError> {
        if !self.bus.runtime_state_supported() {
            return Err(save_state::SaveStateError::Unsupported);
        }
        let active_resources = self.bus.save_state_resources()?;
        let active_media = self.bus.save_state_media()?;
        save_state::restore_machine_state(
            self,
            blob,
            active_resources,
            active_media,
            512 << 20,
            |machine| {
                Ok(Pc98RuntimeState {
                    cpu: machine.cpu.capture_runtime_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.cpu.restore_runtime_state(state.cpu)?;
                machine.bus.restore_runtime_state(state.bus)?;
                machine
                    .bus
                    .set_cpu_protected_mode_enabled(machine.cpu.cr0() & 1 != 0);
                Ok(())
            },
        )
    }
}

impl<C: Pc98RuntimeCpu, T: TraceSink> common::Machine for Pc98Machine<C, T> {
    fn capture_state(&mut self) -> Result<common::MachineStateBlob, common::SaveStateError> {
        self.capture_machine_blob()
    }

    fn restore_state(
        &mut self,
        blob: &common::MachineStateBlob,
    ) -> Result<(), common::SaveStateError> {
        self.restore_machine_blob(blob)
    }

    fn set_host_date_time_provider(&mut self, provider: common::HostDateTimeProvider) {
        self.bus.set_host_date_time_provider(provider);
    }

    fn startup_capabilities(&self) -> common::StartupCapabilities {
        common::StartupCapabilities {
            hard_disk: true,
            printer: true,
            mt32: true,
            sc55: true,
            ..Default::default()
        }
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc98Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.bus.shutdown_requested()
    }

    fn display_framebuffer(&self) -> &[u8] {
        self.bus.display_framebuffer()
    }

    fn display_dimensions(&self) -> (u32, u32) {
        self.bus.display_dimensions()
    }

    fn push_keyboard_scancode(&mut self, code: u8) {
        self.bus.push_keyboard_scancode(code);
    }

    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, middle: bool) {
        self.bus.set_mouse_buttons(left, right, middle);
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn cd_audio_status(&self) -> Option<common::CdAudioStatus> {
        self.bus.cd_audio_status()
    }

    fn font_rom_data(&self) -> &[u8] {
        self.bus.font_rom_data()
    }

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        insert_floppy_impl(&mut self.bus, drive, path)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    fn insert_hdd(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let image = device::disk::load_hdd_image(path, &data)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        validate_hdd_for_model(self.bus.machine_model(), &image.geometry)?;
        let description = format!(
            "HDD{}: {}C/{}H/{}S ({}) from {}",
            drive + 1,
            image.geometry.cylinders,
            image.geometry.heads,
            image.geometry.sectors_per_track,
            image.format_name(),
            path.display(),
        );
        self.bus.insert_hdd(drive, image, Some(path.to_path_buf()));
        Ok(description)
    }

    fn attach_printer(&mut self, path: &std::path::Path) -> Result<(), String> {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .map_err(|error| {
                format!("Failed to open printer output {}: {error}", path.display())
            })?;
        self.bus.attach_printer(file);
        Ok(())
    }

    #[cfg(feature = "mt32")]
    fn install_mt32(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        self.bus
            .install_mt32(rom_directory)
            .map_err(|error| error.to_string())
    }

    #[cfg(feature = "sc55")]
    fn install_sc55(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        self.bus
            .install_sc55(rom_directory)
            .map_err(|error| error.to_string())
    }

    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        insert_cdrom_impl(&mut self.bus, path)
    }

    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
    }

    fn flush_floppies(&mut self) {
        self.bus.flush_all_floppies();
    }

    fn flush_hdds(&mut self) {
        self.bus.flush_all_hdds();
    }

    fn flush_printer(&mut self) {
        self.bus.flush_printer();
    }

    fn install_text_extractor(&mut self, extractor: Box<dyn common::TextExtractor>) {
        self.bus.install_text_extractor(extractor);
    }

    fn tick_text_extractor(&mut self) {
        self.bus.tick_text_extractor();
    }
}

#[cfg(test)]
mod save_state_tests {
    use common::{CpuMode, Machine, MachineModel};

    use super::*;

    fn assert_replay<C: Pc98RuntimeCpu>(model: MachineModel, cpu: C) {
        let bus = Pc9801Bus::new(model, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu, bus);
        let initial = machine.capture_state().unwrap();
        machine.run_for(1);
        let expected = machine.capture_state().unwrap();

        machine.restore_state(&initial).unwrap();
        machine.run_for(1);
        let replayed = machine.capture_state().unwrap();
        assert_eq!(replayed.payload(), expected.payload());
    }

    #[test]
    fn every_pc98_model_replays_from_the_machine_root() {
        assert_replay(MachineModel::PC9801F, cpu::I8086::new());
        assert_replay(MachineModel::PC9801VM, cpu::V30::new());
        assert_replay(MachineModel::PC9801VX, cpu::I286::new());
        assert_replay(
            MachineModel::PC9801RS,
            cpu::I386::<{ cpu::CPU_MODEL_386_SX }>::new(),
        );
        assert_replay(
            MachineModel::PC9801RA,
            cpu::I386::<{ cpu::CPU_MODEL_386_DX }>::new(),
        );
        assert_replay(
            MachineModel::PC9821AS,
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
        );
        assert_replay(
            MachineModel::PC9821AP,
            cpu::I386::<{ cpu::CPU_MODEL_486_DX }>::new(),
        );
    }

    #[test]
    fn corrupt_machine_payload_does_not_mutate_the_machine() {
        let bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let valid = machine.capture_state().unwrap();
        let before = valid.payload().to_vec();
        let mut corrupt_payload = before.clone();
        corrupt_payload.truncate(corrupt_payload.len() / 2);
        let corrupt = valid.with_payload(corrupt_payload).unwrap();

        assert!(machine.restore_state(&corrupt).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), before);
    }

    #[test]
    fn machine_root_restores_directly_from_memory() {
        let bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let snapshot = machine.capture_state().unwrap();
        machine.run_for(257);
        machine.restore_state(&snapshot).unwrap();
        assert_eq!(machine.capture_state().unwrap(), snapshot);
    }

    #[test]
    fn hle_dos_configuration_replays_from_the_machine_root() {
        let mut bus = Pc9801Bus::new(MachineModel::PC9801VM, CpuMode::High, 48_000);
        bus.enable_neetan_dos();
        let mut machine = Pc98Machine::new(cpu::V30::new(), bus);
        let initial = machine.capture_state().unwrap();
        machine.run_for(1);
        let expected = machine.capture_state().unwrap();

        machine.restore_state(&initial).unwrap();
        machine.run_for(1);
        let replayed = machine.capture_state().unwrap();
        assert_eq!(replayed.payload(), expected.payload());
    }
}
