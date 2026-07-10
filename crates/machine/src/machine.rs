use common::{Bus, Cpu, likely, unlikely};

use crate::{CpuState, MachineState, NoTracing, Pc9801Bus, Tracing};

/// Generic PC-9801 machine: a CPU wired to the shared PC-9801 bus.
pub struct Machine<C: Cpu, T: Tracing = NoTracing> {
    /// The CPU.
    pub cpu: C,
    /// The system bus.
    pub bus: Pc9801Bus<T>,
}

impl<C: Cpu, T: Tracing> Machine<C, T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(cpu: C, bus: Pc9801Bus<T>) -> Self {
        Self { cpu, bus }
    }

    /// Runs the machine for up to `budget` CPU cycles.
    ///
    /// When the CPU halts, advances time to the next scheduled event
    /// so that timer interrupts can fire and wake the CPU.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let mut total = 0u64;
        while likely(total < budget) {
            self.bus
                .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);
            let remaining = budget - total;
            let ran = self.cpu.run_for(remaining, &mut self.bus);
            total += ran;
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

            // CPU cores may complete the current instruction and return more
            // cycles than the requested budget slice. In that case this
            // invocation is done; do not enter HLT event-advance logic with a
            // wrapped `budget - total`.
            if unlikely(total >= budget) {
                break;
            }

            if unlikely(self.cpu.halted()) {
                let current = self.bus.current_cycle();
                let remaining = budget.saturating_sub(total);

                if let Some(event_cycle) = self.bus.next_event_cycle() {
                    if event_cycle <= current {
                        // Event already due: process it and retry.
                        self.bus.set_current_cycle(current);
                        continue;
                    } else if event_cycle <= current + remaining {
                        // Event within budget: advance to it and try to wake CPU.
                        let idle = event_cycle - current;
                        self.bus.set_current_cycle(event_cycle);
                        total += idle;
                        self.bus
                            .set_cpu_protected_mode_enabled(self.cpu.cr0() & 1 != 0);
                        let retry = self.cpu.run_for(1, &mut self.bus);
                        if retry == 0 && self.cpu.halted() {
                            continue;
                        }
                        total += retry;
                    } else {
                        // Event beyond budget: advance time to end of budget
                        // (CPU idle) so successive calls make progress.
                        self.bus.set_current_cycle(current + remaining);
                        total += remaining;
                        break;
                    }
                } else {
                    // No events scheduled: advance time to end of budget.
                    self.bus.set_current_cycle(current + remaining);
                    total += remaining;
                    break;
                }
            }
        }

        total
    }
}

/// PC-9801F machine type (8086 CPU at 5 / 8 MHz, basic µPD7220, 20-bit address space).
pub type Pc9801F = Machine<cpu::I8086>;

/// PC-9801VM machine type (V30 CPU at 8 / 10 MHz).
pub type Pc9801Vm = Machine<cpu::VX0>;

/// PC-9801VX machine type (80286 CPU at 8 / 10 MHz).
pub type Pc9801Vx = Machine<cpu::I286>;

/// PC-9801RA machine type (80386 SX CPU at 16 / 20 MHz).
pub type Pc9801Ra = Machine<cpu::I386>;

/// PC-9821AS machine type (486DX CPU at 33 MHz, IDE, PEGC).
pub type Pc9821As = Machine<cpu::I386<{ cpu::CPU_MODEL_486 }>>;

/// PC-9821AP machine type (486DX2 CPU at 66 MHz, IDE, PEGC).
pub type Pc9821Ap = Machine<cpu::I386<{ cpu::CPU_MODEL_486 }>>;

impl<T: Tracing> Machine<cpu::I8086, T> {
    /// Captures the full machine state.
    pub fn save_state(&self) -> MachineState {
        self.bus.save_state(CpuState::I8086(self.cpu.state.clone()))
    }

    /// Restores the machine from a previously saved state.
    pub fn load_state(&mut self, state: &MachineState) {
        assert!(matches!(state.cpu, CpuState::I8086(_)));
        if let CpuState::I8086(ref cpu_state) = state.cpu {
            self.cpu.load_state(cpu_state);
        }
        self.bus.load_peripherals(state);
    }
}

impl<T: Tracing> Machine<cpu::VX0, T> {
    /// Captures the full machine state.
    pub fn save_state(&self) -> MachineState {
        self.bus.save_state(CpuState::V30(self.cpu.state.clone()))
    }

    /// Restores the machine from a previously saved state.
    pub fn load_state(&mut self, state: &MachineState) {
        assert!(matches!(state.cpu, CpuState::V30(_)));
        if let CpuState::V30(ref cpu_state) = state.cpu {
            self.cpu.load_state(cpu_state);
        }
        self.bus.load_peripherals(state);
    }
}

impl<T: Tracing> Machine<cpu::I286, T> {
    /// Captures the full machine state.
    pub fn save_state(&self) -> MachineState {
        self.bus.save_state(CpuState::I286(self.cpu.state.clone()))
    }

    /// Restores the machine from a previously saved state.
    pub fn load_state(&mut self, state: &MachineState) {
        assert!(matches!(state.cpu, CpuState::I286(_)));
        if let CpuState::I286(ref cpu_state) = state.cpu {
            self.cpu.load_state(cpu_state);
        }
        self.bus.load_peripherals(state);
    }
}

impl<const CPU_MODEL: u8, T: Tracing> Machine<cpu::I386<CPU_MODEL>, T> {
    /// Captures the full machine state.
    pub fn save_state(&self) -> MachineState {
        self.bus.save_state(CpuState::I386(self.cpu.state.clone()))
    }

    /// Restores the machine from a previously saved state.
    pub fn load_state(&mut self, state: &MachineState) {
        assert!(matches!(state.cpu, CpuState::I386(_)));
        if let CpuState::I386(ref cpu_state) = state.cpu {
            self.cpu.load_state(cpu_state);
        }
        self.bus.load_peripherals(state);
    }
}

fn insert_floppy_impl<T: Tracing>(
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

fn insert_cdrom_impl<T: Tracing>(
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

impl<C: Cpu, T: Tracing> common::Machine for Machine<C, T> {
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
        Machine::run_for(self, budget)
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
