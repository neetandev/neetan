//! Runnable X68000 CPU and motherboard.

use common::{Bus, CpuM68000, CpuMode, JoystickState, Machine, NoTrace, TraceSink};
use cpu_68k::M68000;

use crate::{LoadedRoms, X68kBus, X68kModel};

/// Default audio sample rate.
const DEFAULT_SAMPLE_RATE: u32 = 48_000;

save_state::runtime_state! {
/// Machine-root state for one X68000 snapshot.
#[derive(Clone)]
struct X68kRuntimeState {
    cpu: cpu_68k::M68000RuntimeState,
    bus: crate::bus::X68kBusState,
}}

/// A runnable X68000 machine with an MC68000 and motherboard bus.
pub struct X68kMachine<T: TraceSink = NoTrace> {
    /// Motorola MC68000 CPU.
    pub cpu: Box<M68000>,
    /// X68000 motherboard bus.
    pub bus: X68kBus<T>,
}

impl<T: TraceSink> X68kMachine<T> {
    /// Builds a machine around a configured CPU and bus.
    pub fn new(cpu: Box<M68000>, bus: X68kBus<T>) -> Self {
        Self { cpu, bus }
    }

    /// Builds a reset CPU around an initialized motherboard bus.
    pub fn from_bus(model: X68kModel, cpu_mode: CpuMode, bus: X68kBus<T>) -> Self {
        let mut cpu = Box::new(M68000::new(model.cpu_clock_hz(cpu_mode)));
        cpu.reset();
        Self { cpu, bus }
    }

    /// Mounts a hard-disk image into `slot` (SASI or SCSI IDs 0 and 1).
    pub fn insert_hdd(
        &mut self,
        slot: usize,
        image: device::disk::HddImage,
        path: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        self.bus.insert_hdd(slot, image, path)
    }

    /// Ejects and flushes the hard disk in `slot`.
    pub fn eject_hdd(&mut self, slot: usize) {
        self.bus.eject_hdd(slot);
    }

    /// Installs the CZ-6BM1 MIDI board with transmit-byte capture enabled.
    pub fn install_midi_card(&mut self) {
        self.bus.install_midi_card();
    }

    /// Copies captured MIDI into `target` and returns the number of bytes written.
    pub fn flush_midi_into(&mut self, target: &mut [u8]) -> usize {
        self.bus.flush_midi_into(target)
    }

    /// Installs a Roland MT-32 sound module driven by the CZ-6BM1 card.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.bus.install_mt32(rom_directory)
    }

    /// Installs a Roland SC-55 sound module driven by the CZ-6BM1 card.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.bus.install_sc55(rom_directory)
    }

    /// Advances the CPU by approximately `budget` input cycles.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let slice_end = self
                .bus
                .next_event_cycle()
                .unwrap_or(target_cycle)
                .min(target_cycle);
            let slice_cycles = slice_end.saturating_sub(current_cycle).max(1);
            let ran_cycles = self.cpu.run_for(slice_cycles, &mut self.bus);
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
            if self.bus.current_cycle() >= slice_end {
                self.bus.process_due_events();
            }
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    fn capture_machine_blob(
        &mut self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = X68kRuntimeState {
            cpu: self.cpu.capture_state(),
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
        let active_resources = self.bus.save_state_resources()?;
        let active_media = self.bus.save_state_media()?;
        save_state::restore_machine_state(
            self,
            blob,
            active_resources,
            active_media,
            128 << 20,
            |machine| {
                Ok(X68kRuntimeState {
                    cpu: machine.cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.cpu.restore_state(state.cpu)?;
                machine.bus.restore_runtime_state(state.bus)
            },
        )
    }
}

impl X68kMachine<NoTrace> {
    /// Builds and resets an untraced machine with an explicit main-RAM size.
    pub fn with_main_ram_size(
        model: X68kModel,
        cpu_mode: CpuMode,
        roms: LoadedRoms,
        main_ram_size: usize,
    ) -> Result<Self, String> {
        let bus =
            X68kBus::with_main_ram_size(model, cpu_mode, roms, DEFAULT_SAMPLE_RATE, main_ram_size)?;
        Ok(Self::from_bus(model, cpu_mode, bus))
    }
}

impl<T: TraceSink> Machine for X68kMachine<T> {
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
            mt32: true,
            sc55: true,
            ..Default::default()
        }
    }

    /// Returns the CPU clock.
    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.cpu.clock_hz())
    }

    /// Runs the machine for a cycle budget.
    fn run_for(&mut self, budget: u64) -> u64 {
        X68kMachine::run_for(self, budget)
    }

    /// Reports guest shutdown state.
    fn shutdown_requested(&self) -> bool {
        self.bus.shutdown_requested()
    }

    /// Returns the completed framebuffer.
    fn display_framebuffer(&self) -> &[u8] {
        self.bus.display_framebuffer()
    }

    /// Returns the completed frame dimensions.
    fn display_dimensions(&self) -> (u32, u32) {
        self.bus.display_dimensions()
    }

    /// Accepts a keyboard code.
    fn push_keyboard_scancode(&mut self, code: u8) {
        self.bus.push_keyboard_scancode(code);
    }

    /// Accumulates host mouse movement for the SCC mouse.
    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    /// Updates the held SCC mouse buttons.
    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        self.bus.set_mouse_buttons(left, right);
    }

    /// Updates one of the two PPI joystick ports.
    fn set_joystick(&mut self, index: usize, state: JoystickState) {
        self.bus.set_joystick(index, state);
    }

    /// Generates mixed motherboard audio.
    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    /// Returns the character-generator ROM.
    fn font_rom_data(&self) -> &[u8] {
        self.bus.cgrom_data()
    }

    /// Loads and mounts a floppy image into `drive`.
    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()))?;
        Ok(description)
    }

    /// Ejects and flushes the floppy in `drive`.
    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    /// Flushes every mounted floppy to its backing file.
    fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }

    /// Flushes every mounted hard disk to its backing file.
    fn flush_hdds(&mut self) {
        self.bus.flush_hdds();
    }

    fn insert_hdd(&mut self, slot: usize, path: &std::path::Path) -> Result<String, String> {
        let extension_is_hdf = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("hdf"));
        if !extension_is_hdf {
            return Err(format!(
                "X68000 hard disks must be headerless .hdf images; got {}",
                path.display(),
            ));
        }
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let sector_size = self.bus.model().storage_controller().sector_size();
        let image = device::disk::load_x68k_hdf(data, sector_size)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        let description = format!(
            "{} HDD unit {slot}: {} sectors from {}",
            self.bus.model(),
            image.geometry.total_sectors(),
            path.display(),
        );
        X68kMachine::insert_hdd(self, slot, image, Some(path.to_path_buf()))?;
        Ok(description)
    }

    #[cfg(feature = "mt32")]
    fn install_mt32(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        X68kMachine::install_mt32(self, rom_directory).map_err(|error| error.to_string())
    }

    #[cfg(feature = "sc55")]
    fn install_sc55(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        X68kMachine::install_sc55(self, rom_directory).map_err(|error| error.to_string())
    }

    /// Loads and inserts a CD-ROM image into the internal SCSI drive.
    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        let (image, description) = device::cdrom::load_cd_image(path)?;
        self.bus.insert_cdrom(image)?;
        Ok(description)
    }

    /// Ejects the CD-ROM media.
    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
    }
}

#[cfg(test)]
mod tests {
    use common::{CpuMode, TraceAccessKind, TraceEvent, TraceSink};

    use super::*;

    fn roms() -> LoadedRoms {
        let mut ipl = vec![0; 0x20000];
        ipl[0x10000..0x10008].copy_from_slice(&[0, 0xBF, 0xF0, 0, 0, 0xFF, 0, 8]);
        LoadedRoms {
            model: X68kModel::X68000,
            cgrom: vec![0; 0xC0000],
            ipl,
            internal_scsi: None,
            uses_compatibility_scsi: false,
        }
    }

    #[derive(Default)]
    struct YieldOnScheduled {
        saw_scheduled: bool,
        fetch_after_scheduled: bool,
    }

    impl TraceSink for YieldOnScheduled {
        fn trace(&mut self, _context: common::TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Scheduled { .. } => self.saw_scheduled = true,
                TraceEvent::Access(access)
                    if self.saw_scheduled && access.kind == TraceAccessKind::Fetch =>
                {
                    self.fetch_after_scheduled = true;
                }
                _ => {}
            }
        }

        fn yield_requested(&self) -> bool {
            self.saw_scheduled
        }
    }

    #[test]
    fn scheduled_trace_yield_prevents_a_later_fetch() {
        let model = X68kModel::X68000;
        let cpu_mode = CpuMode::High;
        let mut bus = X68kBus::new_with_trace_sink(
            model,
            cpu_mode,
            roms(),
            48_000,
            YieldOnScheduled::default(),
        )
        .unwrap();
        bus.schedule_trace_test_event();
        let mut machine = X68kMachine::from_bus(model, cpu_mode, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn fresh_machine_outputs_black_frame_and_silence() {
        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, roms());
        assert_eq!(machine.display_dimensions(), (768, 512));
        assert!(
            machine
                .display_framebuffer()
                .chunks_exact(4)
                .all(|pixel| pixel == [0, 0, 0, 0xFF])
        );
        let mut audio = [1.0; 8];
        assert_eq!(machine.generate_audio_samples(1.0, &mut audio), 8);
        assert_eq!(audio, [0.0; 8]);
    }

    #[test]
    fn insert_floppy_routes_containers_by_extension_and_bounds_the_drive() {
        use common::Machine;
        use device::floppy::{D88Disk, D88MediaType, D88Sector, FloppyImage};

        let sector = D88Sector {
            cylinder: 0,
            head: 0,
            record: 1,
            size_code: 1,
            sector_count: 1,
            mfm_flag: 0x00,
            deleted: 0x00,
            status: 0x00,
            reserved: [0; 5],
            data: vec![0x5A; 256],
            source_offset: None,
        };
        let disk = D88Disk::from_tracks(
            String::from("PROBE"),
            false,
            D88MediaType::Disk2HD,
            vec![Some(vec![sector])],
        );
        let path = std::env::temp_dir().join("neetan_machine_insert.d88");
        std::fs::write(&path, FloppyImage::from_d88(disk).to_bytes()).unwrap();

        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, roms());
        let label = machine.insert_floppy(0, &path).unwrap();
        assert!(
            label.contains("D88"),
            "label reports the container: {label}"
        );
        assert!(
            machine.insert_floppy(2, &path).is_err(),
            "the X68000 only installs drives 0 and 1"
        );
        machine.flush_floppies();
        machine.eject_floppy(0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn synthetic_ipl_executes_through_the_real_cpu_bus() {
        let mut loaded = roms();
        loaded.ipl[0x10000..0x10008].copy_from_slice(&[0, 0xBF, 0xF0, 0, 0, 0xFF, 0, 8]);
        let program = [0x13FC_u16, 0x00A5, 0x0000, 0x0100, 0x4E72, 0x2700];
        for (index, word) in program.into_iter().enumerate() {
            let offset = 0x10008 + index * 2;
            loaded.ipl[offset..offset + 2].copy_from_slice(&word.to_be_bytes());
        }
        let mut machine: X68kMachine =
            crate::bus::test_support::machine(X68kModel::X68000, CpuMode::High, loaded);
        machine.run_for(200);
        assert_eq!(machine.bus.ram_byte(0x100), Some(0xA5));
    }
}
