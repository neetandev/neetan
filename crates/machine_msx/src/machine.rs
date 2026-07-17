//! Top-level MSX machine.
//!
//! One Z80 drives one bus paced by a monotonic cycle in normal CPU T-states.

use common::{CpuZ80, NoTrace, TraceSink};

use crate::{MainBusView, MsxBus};

save_state::runtime_state! {
/// Machine-root state for one MSX family snapshot.
#[derive(Clone)]
struct MsxRuntimeState {
    cpu: cpu::Z80State,
    bus: crate::bus::MsxBusState,
}}

/// MSX machine with one Z80 and one system bus.
pub struct MsxMachine<T: TraceSink = NoTrace> {
    /// Main Z80.
    pub main_cpu: cpu::Z80,
    /// System bus.
    pub bus: MsxBus<T>,
}

/// Builds an untraced machine around a configured bus.
pub fn build_untraced_machine(bus: MsxBus<NoTrace>) -> Box<dyn common::Machine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(MsxMachine::new(main_cpu, bus))
}

impl<T: TraceSink> MsxMachine<T> {
    /// Creates a machine from a Z80 and bus.
    pub const fn new(main_cpu: cpu::Z80, bus: MsxBus<T>) -> Self {
        Self { main_cpu, bus }
    }

    /// Runs for up to `budget` normal-Z80 cycles, sliced at scheduled events.
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
            let ran_cycles = {
                let mut view = MainBusView { bus: &mut self.bus };
                self.main_cpu.run_for(slice_cycles, &mut view)
            };
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }

            if self.bus.current_cycle() >= slice_end {
                self.bus.process_events();
                if T::ENABLED && self.bus.tracer().yield_requested() {
                    break;
                }
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    /// Captures a complete machine blob with resource and media manifests.
    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        save_state::capture_machine_state(
            MsxRuntimeState {
                cpu: self.main_cpu.capture_state(),
                bus: self.bus.capture_runtime_state()?,
            },
            self.bus.save_state_resources()?,
            self.bus.save_state_media()?,
        )
    }

    /// Restores a complete machine blob transactionally.
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
            64 << 20,
            |machine| {
                Ok(MsxRuntimeState {
                    cpu: machine.main_cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.main_cpu.restore_state(state.cpu)?;
                machine.bus.restore_runtime_state(state.bus)
            },
        )
    }
}

impl<T: TraceSink> common::Machine for MsxMachine<T> {
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
            cartridge: true,
            cassette: true,
            ..common::StartupCapabilities::default()
        }
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        MsxMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
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

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        if index < 2 {
            self.bus.set_joystick(
                index,
                crate::MsxJoystickState {
                    up: state.up,
                    down: state.down,
                    left: state.left,
                    right: state.right,
                    trigger_a: state.trigger1,
                    trigger_b: state.trigger2,
                },
            );
        }
    }

    fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.bus.push_mouse_delta(delta_x, delta_y);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        self.bus.set_mouse_buttons(left, right);
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn font_rom_data(&self) -> &[u8] {
        &[]
    }

    fn insert_cartridge(&mut self, path: &std::path::Path) -> Result<String, String> {
        let image = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let info = self
            .bus
            .insert_cartridge_from_path(0, &image, path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let warning = info
            .warning
            .map_or_else(String::new, |warning| format!("; warning: {warning}"));
        Ok(format!(
            "{} ({} bytes, {}){warning}",
            path.display(),
            image.len(),
            info.mapper
        ))
    }

    fn eject_cartridge(&mut self) {
        if let Err(error) = self.bus.eject_cartridge(0) {
            common::error!("Failed to eject MSX cartridge: {error}");
        }
    }

    fn insert_cassette(&mut self, path: &std::path::Path) -> Result<String, String> {
        let image = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        self.bus
            .insert_cassette_from_path(extension, &image, path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(format!("{} ({} bytes)", path.display(), image.len()))
    }

    fn eject_cassette(&mut self) {
        self.bus.eject_cassette();
    }

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        if drive >= usize::from(self.bus.model().drive_count()) {
            return Err(format!("MSX drive {drive} is not present"));
        }
        let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus.insert_floppy(drive, image, path.to_path_buf());
        Ok(description)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }

    fn flush_cartridges(&mut self) {
        if let Err(error) = self.bus.flush_cartridges() {
            common::error!("Failed to flush MSX cartridge data: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{Machine, TraceAccessKind, TraceContext, TraceEvent, TraceSink};

    use super::*;
    use crate::MsxModel;

    #[derive(Default)]
    struct YieldOnScheduled {
        saw_scheduled: bool,
        fetch_after_scheduled: bool,
    }

    impl TraceSink for YieldOnScheduled {
        fn trace(&mut self, _context: TraceContext, event: TraceEvent<'_>) {
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
        let mut bus =
            MsxBus::new_with_trace_sink(MsxModel::Msx, 48_000, YieldOnScheduled::default());
        bus.load_synthetic_program(&[0xC3, 0x00, 0x00]).unwrap();
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = MsxMachine::new(main_cpu, bus);
        machine.run_for(1_000);
        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn halted_cpu_advances_through_scheduler_events() {
        let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
        bus.load_synthetic_program(&[0x76]).unwrap();
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = MsxMachine::new(main_cpu, bus);
        machine.run_for(1_000);
        assert!(machine.bus.current_cycle() >= 1_000);
        assert!(machine.bus.completed_scanlines() >= 4);
    }

    fn state_test_machine(model: MsxModel) -> MsxMachine {
        let mut bus = MsxBus::new(model, 48_000);
        bus.load_synthetic_program(&[
            0x3C, // INC A
            0x32, 0x00, 0x40, // LD (0x4000),A
            0xD3, 0x98, // OUT (0x98),A
            0xC3, 0x00, 0x00, // JP 0
        ])
        .unwrap();
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        MsxMachine::new(main_cpu, bus)
    }

    #[test]
    fn every_msx_model_replays_from_a_runtime_state() {
        for model in MsxModel::ALL {
            let mut machine = state_test_machine(model);
            machine.run_for(20_000);
            let initial = machine.capture_state().unwrap();

            machine.push_keyboard_scancode(0x1D);
            machine.push_mouse_delta(7, -5);
            machine.run_for(30_000);
            let mut expected_audio = vec![0.0; 2_048];
            machine.generate_audio_samples(1.0, &mut expected_audio);
            let expected = machine.capture_state().unwrap();

            machine.restore_state(&initial).unwrap();
            machine.push_keyboard_scancode(0x1D);
            machine.push_mouse_delta(7, -5);
            machine.run_for(30_000);
            let mut replayed_audio = vec![0.0; 2_048];
            machine.generate_audio_samples(1.0, &mut replayed_audio);
            let replayed = machine.capture_state().unwrap();

            assert_eq!(replayed.payload(), expected.payload(), "{model}");
            assert_eq!(replayed_audio, expected_audio, "{model}");
        }
    }

    #[test]
    fn corrupt_state_does_not_mutate_the_running_machine() {
        let mut machine = state_test_machine(MsxModel::Msx2);
        machine.run_for(10_000);
        let valid = machine.capture_state().unwrap();
        let corrupt = valid
            .with_payload(valid.payload()[..valid.payload().len() / 2].to_vec())
            .unwrap();

        assert!(machine.restore_state(&corrupt).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), valid.payload());
    }

    #[test]
    fn model_mismatch_is_rejected() {
        let snapshot = state_test_machine(MsxModel::Msx).capture_state().unwrap();
        let mut other = state_test_machine(MsxModel::Msx2);
        assert!(other.restore_state(&snapshot).is_err());
    }

    #[test]
    fn cartridge_mapper_state_rewinds_and_identity_is_checked() {
        use common::Bus as _;

        let mut image = vec![0; 0x1_0000];
        image[..6].copy_from_slice(&[0x32, 0x00, 0x50, 0x32, 0x00, 0x90]);
        for (bank, bytes) in image.chunks_exact_mut(0x2000).enumerate() {
            bytes.fill(bank as u8);
        }
        image[..6].copy_from_slice(&[0x32, 0x00, 0x50, 0x32, 0x00, 0x90]);

        let mut machine = state_test_machine(MsxModel::Msx);
        machine.bus.insert_cartridge(0, &image).unwrap();
        {
            let mut view = MainBusView {
                bus: &mut machine.bus,
            };
            view.io_write_byte(0xAB, 0x82);
            view.io_write_byte(0xA8, 0x55);
        }
        machine.bus.poke_byte(0x9000, 5);
        assert_eq!(machine.bus.peek_byte(0x8000), 5);
        let snapshot = machine.capture_state().unwrap();

        machine.bus.poke_byte(0x9000, 2);
        assert_eq!(machine.bus.peek_byte(0x8000), 2);
        machine.restore_state(&snapshot).unwrap();
        assert_eq!(machine.bus.peek_byte(0x8000), 5);

        let mut different = state_test_machine(MsxModel::Msx);
        image[0x1234] ^= 0xFF;
        different.bus.insert_cartridge(0, &image).unwrap();
        assert!(different.restore_state(&snapshot).is_err());
    }

    #[test]
    fn active_scc_with_audio_backlog_restores_transactionally() {
        use common::Bus as _;

        let mut image = vec![0; 0x1_0000];
        image[..6].copy_from_slice(&[0x32, 0x00, 0x50, 0x32, 0x00, 0x90]);
        let mut machine = state_test_machine(MsxModel::Msx);
        machine.bus.insert_cartridge(0, &image).unwrap();
        {
            let mut view = MainBusView {
                bus: &mut machine.bus,
            };
            view.io_write_byte(0xAB, 0x82);
            view.io_write_byte(0xA8, 0x55);
        }
        for (address, value) in [
            (0x9000, 0x3F),
            (0x9800, 0x40),
            (0x9880, 9),
            (0x9881, 0),
            (0x988A, 15),
            (0x988F, 1),
        ] {
            machine.bus.poke_byte(address, value);
        }
        machine.run_for(u64::from(machine.bus.cpu_clock_hz()));
        let mut small_output = [0.0; 20];
        machine.generate_audio_samples(1.0, &mut small_output);

        let snapshot = machine.capture_state().unwrap();
        machine.run_for(10_000);
        machine.restore_state(&snapshot).unwrap();
        assert_eq!(
            machine.capture_state().unwrap().payload(),
            snapshot.payload()
        );
    }
}
