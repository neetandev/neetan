//! Top-level PC-6000 machine.
//!
//! A single-Z80 machine: one CPU driving one bus, paced by a monotonic
//! `current_cycle` in main-clock units.

use common::{CpuZ80, NoTrace, TraceSink};

use crate::bus::{MainBusView, Pc6000Bus};

save_state::runtime_state! {
/// Machine-root state for one PC-6000 family snapshot.
#[derive(Clone)]
struct Pc6000RuntimeState {
    cpu: cpu::Z80State,
    bus: crate::bus::Pc6000BusState,
}}

/// PC-6000 machine: the main Z80 sharing one bus.
pub struct Pc6000Machine<T: TraceSink = NoTrace> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// System bus.
    pub bus: Pc6000Bus<T>,
}

/// Builds an untraced PC-6000 machine around a configured bus.
pub fn build_untraced_machine(bus: Pc6000Bus<NoTrace>) -> Box<dyn common::Machine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(Pc6000Machine::new(main_cpu, bus))
}

impl<T: TraceSink> Pc6000Machine<T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(main_cpu: cpu::Z80, bus: Pc6000Bus<T>) -> Self {
        Self { main_cpu, bus }
    }

    /// Runs the main CPU for up to `budget` main-clock cycles, returning the
    /// cycles actually advanced. Execution is sliced to the next scheduled
    /// event so timer and frame interrupts fire promptly; a halted CPU idles
    /// forward to the next event to keep the scheduler clock moving.
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
                .scheduler
                .next_event_cycle()
                .unwrap_or(target_cycle)
                .min(target_cycle);

            if self.bus.cpu_stalled() {
                // The video circuit holds the bus; the CPU idles to the next
                // event (the bus-request release) without executing.
                self.bus.set_current_cycle(slice_end);
            } else {
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

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = Pc6000RuntimeState {
            cpu: self.main_cpu.capture_state(),
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
            64 << 20,
            |machine| {
                Ok(Pc6000RuntimeState {
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

impl<T: TraceSink> common::Machine for Pc6000Machine<T> {
    fn capture_state(&mut self) -> Result<common::MachineStateBlob, common::SaveStateError> {
        self.capture_machine_blob()
    }

    fn restore_state(
        &mut self,
        blob: &common::MachineStateBlob,
    ) -> Result<(), common::SaveStateError> {
        self.restore_machine_blob(blob)
    }

    fn startup_capabilities(&self) -> common::StartupCapabilities {
        common::StartupCapabilities {
            cassette: true,
            ..common::StartupCapabilities::default()
        }
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc6000Machine::run_for(self, budget)
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
        if index == 0 {
            self.bus.set_joystick(state);
        }
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn font_rom_data(&self) -> &[u8] {
        self.bus.font_rom_data()
    }

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()));
        Ok(description)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
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

    fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }
}

#[cfg(test)]
mod tests {
    use common::{CpuZ80, Machine, TraceAccessKind, TraceEvent, TraceSink};

    use super::*;
    use crate::{config::Pc6000Model, rom::LoadedRoms};

    fn loaded_roms_with_boot(model: Pc6000Model, boot: Vec<u8>) -> LoadedRoms {
        LoadedRoms {
            model,
            basic: Some(boot),
            system_rom1: None,
            system_rom2: None,
            sub_rom: None,
            cg_base: None,
            cg_ext: None,
            cg_sr: None,
            kanji: None,
            voice: None,
        }
    }

    fn machine_with_boot(model: Pc6000Model, boot: Vec<u8>) -> Pc6000Machine {
        let mut bus = Pc6000Bus::new(model, 48_000);
        bus.load_roms(&loaded_roms_with_boot(model, boot));
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        Pc6000Machine::new(main_cpu, bus)
    }

    #[test]
    fn cpu_executes_rom() {
        // A boot ROM of NOPs (0x00) lets the CPU stream forward from the reset
        // vector without trapping.
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        let start_pc = machine.main_cpu.pc();

        let ran = machine.run_for(100);

        assert!(ran >= 100, "the cycle budget is consumed");
        assert_ne!(
            machine.main_cpu.pc(),
            start_pc,
            "the CPU advances through ROM"
        );
    }

    #[derive(Default)]
    struct YieldOnFirstEvent {
        yield_requested: bool,
    }

    impl TraceSink for YieldOnFirstEvent {
        fn trace(&mut self, _context: common::TraceContext, _event: TraceEvent<'_>) {
            self.yield_requested = true;
        }

        fn yield_requested(&self) -> bool {
            self.yield_requested
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

    #[derive(Default)]
    struct YieldOnPresentation {
        presentation: Option<common::TracePresentation>,
    }

    impl TraceSink for YieldOnPresentation {
        fn trace(&mut self, _context: common::TraceContext, event: TraceEvent<'_>) {
            if let TraceEvent::Presentation(presentation) = event {
                self.presentation = Some(presentation);
            }
        }

        fn yield_requested(&self) -> bool {
            self.presentation.is_some()
        }
    }

    #[test]
    fn trace_yield_finishes_the_current_instruction() {
        let model = Pc6000Model::Pc6001;
        let mut bus = Pc6000Bus::new_with_trace_sink(model, 48_000, YieldOnFirstEvent::default());
        bus.load_roms(&loaded_roms_with_boot(model, vec![0x00; 0x1000]));
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = Pc6000Machine::new(main_cpu, bus);
        let start_pc = machine.main_cpu.pc();

        let ran = machine.run_for(100);

        assert!(ran > 0);
        assert_eq!(machine.main_cpu.pc(), start_pc.wrapping_add(1));
        assert!(machine.bus.tracer().yield_requested());
        assert_eq!(machine.run_for(100), 0);
        assert_eq!(machine.main_cpu.pc(), start_pc.wrapping_add(1));
    }

    #[test]
    fn scheduled_trace_yield_prevents_a_later_fetch() {
        let model = Pc6000Model::Pc6001;
        let mut bus = Pc6000Bus::new_with_trace_sink(model, 48_000, YieldOnScheduled::default());
        bus.load_roms(&loaded_roms_with_boot(model, vec![0x00; 0x1000]));
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = Pc6000Machine::new(main_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn presentation_trace_is_emitted_after_frame_publication() {
        let model = Pc6000Model::Pc6001;
        let mut bus = Pc6000Bus::new_with_trace_sink(model, 48_000, YieldOnPresentation::default());
        bus.load_roms(&loaded_roms_with_boot(model, vec![0x00; 0x1000]));
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = Pc6000Machine::new(main_cpu, bus);

        let ran = machine.run_for(100_000);

        assert!(ran < 100_000);
        assert_eq!(
            machine.bus.tracer().presentation,
            Some(common::TracePresentation {
                display: common::trace_id::display::MAIN,
                frame: 1,
                width: 256,
                height: 192,
            })
        );
        assert_eq!(machine.bus.display_dimensions(), (256, 192));
        assert_eq!(machine.run_for(1), 0);
    }

    #[test]
    fn audio_pacing_tracks_elapsed_cycles() {
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        machine.run_for(machine.bus.cpu_clock_hz() as u64);
        let mut output = vec![0.0_f32; 200_000];
        let written = machine.generate_audio_samples(1.0, &mut output);
        // One second of main-clock cycles is roughly one second of samples.
        assert!(
            (2 * 47_000..=2 * 49_000).contains(&written),
            "got {written}"
        );
    }

    #[test]
    fn boot_program_renders_a_screen() {
        // The boot program selects the 0xC000 text base. The base text background
        // is a non-black pen, so once a frame renders the framebuffer carries lit
        // pixels.
        let boot = vec![
            0xF3, // DI
            0x3E, 0x00, // LD A, 0x00
            0xD3, 0xB0, // OUT (0xB0), A   ; select 0xC000 base, text mode
            0x18, 0xFE, // JR $
        ];
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, boot);

        let frame = u64::from(machine.bus.cpu_clock_hz()) / 60;
        for _ in 0..4 {
            machine.run_for(frame);
        }

        let framebuffer = machine.display_framebuffer();
        let lit = framebuffer
            .chunks_exact(4)
            .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            .count();
        assert!(lit > 0, "the boot program should render a non-blank screen");
    }

    #[test]
    fn mkii_reports_extended_display_dimensions() {
        // The mkII renders a 320x240 image.
        let machine = machine_with_boot(Pc6000Model::Pc6001Mk2, vec![0x00; 0x1000]);
        assert_eq!(machine.display_dimensions(), (320, 240));
    }

    #[test]
    fn every_model_replays_from_a_runtime_state() {
        for model in [
            Pc6000Model::Pc6001,
            Pc6000Model::Pc6001Mk2,
            Pc6000Model::Pc6601,
            Pc6000Model::Pc6001Mk2Sr,
            Pc6000Model::Pc6601Sr,
        ] {
            let mut machine =
                machine_with_boot(model, vec![0x00; model.work_ram_size().min(0x8000)]);
            machine
                .bus
                .insert_cassette("p6", &[0x10, 0x20, 0x30])
                .unwrap();
            machine.bus.io_write(0xB0, 0x09);
            for (register, value) in [(0x00, 0x40), (0x01, 0x00), (0x08, 0x0F), (0x07, 0x3E)] {
                machine.bus.io_write(0xA0, register);
                machine.bus.io_write(0xA1, value);
            }
            machine.run_for(20_000);
            let initial = machine.capture_state().unwrap();

            machine.push_keyboard_scancode(0x41);
            machine.run_for(30_000);
            let mut expected_audio = vec![0.0; 2048];
            machine.generate_audio_samples(1.0, &mut expected_audio);
            let expected = machine.capture_state().unwrap();

            machine.restore_state(&initial).unwrap();
            machine.push_keyboard_scancode(0x41);
            machine.run_for(30_000);
            let mut replayed_audio = vec![0.0; 2048];
            machine.generate_audio_samples(1.0, &mut replayed_audio);
            let replayed = machine.capture_state().unwrap();

            assert_eq!(replayed.payload(), expected.payload(), "{model}");
            assert_eq!(replayed_audio, expected_audio, "{model}");
        }
    }

    #[test]
    fn corrupt_state_does_not_mutate_the_running_machine() {
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        machine.run_for(10_000);
        let valid = machine.capture_state().unwrap();
        let before = valid.payload().to_vec();
        let corrupt = valid
            .with_payload(valid.payload()[..valid.payload().len() / 2].to_vec())
            .unwrap();

        assert!(machine.restore_state(&corrupt).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), before);
    }

    #[test]
    fn cassette_identity_mismatch_is_rejected_before_restore() {
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        machine.bus.insert_cassette("p6", &[1, 2, 3]).unwrap();
        let snapshot = machine.capture_state().unwrap();
        machine.eject_cassette();
        let before = machine.capture_state().unwrap();

        assert!(machine.restore_state(&snapshot).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), before.payload());
    }

    #[test]
    fn rom_and_model_mismatches_are_rejected() {
        let mut source = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        let snapshot = source.capture_state().unwrap();

        let mut different_rom = machine_with_boot(Pc6000Model::Pc6001, vec![0xFF; 0x1000]);
        assert!(different_rom.restore_state(&snapshot).is_err());

        let mut different_model = machine_with_boot(Pc6000Model::Pc6001Mk2, vec![0x00; 0x8000]);
        assert!(different_model.restore_state(&snapshot).is_err());
    }
}
