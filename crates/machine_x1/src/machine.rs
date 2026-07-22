//! Top-level Sharp X1 machine.
//!
//! A single-Z80 machine: one CPU driving one bus, paced by a monotonic
//! `current_cycle` in main-clock units.

use common::{CpuZ80, HostKey, KeyModifiers, NoTrace, TraceSink};

use crate::bus::{MainBusView, X1Bus};

save_state::runtime_state! {
/// Machine-root state for one Sharp X1 family snapshot.
#[derive(Clone)]
struct X1RuntimeState {
    cpu: cpu::Z80State,
    bus: crate::bus::X1BusState,
}}

/// Sharp X1 machine: the main Z80 sharing one bus.
pub struct X1Machine<T: TraceSink = NoTrace> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// System bus.
    pub bus: X1Bus<T>,
}

/// Builds an untraced X1 machine around a configured bus.
pub fn build_untraced_machine(bus: X1Bus<NoTrace>) -> Box<dyn common::Machine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(X1Machine::new(main_cpu, bus))
}

/// Builds an automated X1 machine around a configured bus.
pub fn build_automated_machine(
    bus: X1Bus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(X1Machine::new(main_cpu, bus))
}

impl common::AutomationDriver for X1Machine<common::tracing::ApplicationTraceSink> {
    fn arm_presentation_yield(&mut self, target_frame: u64) {
        self.bus.tracer().arm_presentation_yield(target_frame);
    }

    fn disarm_presentation_yield(&mut self) {
        self.bus.tracer().disarm_presentation_yield();
    }

    fn epoch_ticks(&self) -> u64 {
        self.bus.current_cycle()
    }

    fn epoch_frames(&self) -> u64 {
        self.bus.presented_frames()
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        X1Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for X1Machine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "x1",
            model: self.bus.model_id(),
            timebase: common::AutomationTimebase {
                ticks_per_second_numerator: numerator,
                ticks_per_second_denominator: denominator,
            },
            audio_sample_rate: self.bus.audio_sample_rate(),
            input: common::InputCapabilities {
                keyboard: true,
                mouse_buttons: 0,
                joystick_ports: 1,
            },
        }
    }

    fn automation_timeline(&self) -> common::AutomationTimeline {
        common::AutomationTimeline {
            epoch_ticks: self.bus.current_cycle() as u128,
            epoch_frames: self.bus.presented_frames() as u128,
            ..common::AutomationTimeline::default()
        }
    }

    fn run_automation(&mut self, request: common::RunRequest) -> common::RunOutcome {
        common::drive_automation(self, request)
    }

    fn inspector(&mut self) -> Option<&mut dyn common::MachineInspector> {
        Some(self)
    }

    fn trace_catalog(&self) -> common::TraceCatalog {
        X1_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the Sharp X1 bus.
const X1_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::X1_DAISY],
    scheduled: common::trace_id::scheduled::X1,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::X1_FDC,
        actions: &[common::trace_action(common::trace_id::action::READ)],
    }],
    providers: &[],
};

impl common::MachineInspector for X1Machine<common::tracing::ApplicationTraceSink> {
    fn processors(&self) -> common::ProcessorList {
        let mut processors = common::ProcessorList::new();
        processors.push(common::inspect::z80_processor("cpu.main"));
        processors
    }

    fn address_spaces(&self) -> common::AddressSpaceList {
        let mut spaces = common::AddressSpaceList::new();
        spaces.push(common::inspect::memory_space(
            "cpu.main.memory",
            16,
            common::ByteOrder::Little,
        ));
        spaces.push(common::inspect::io_space(
            "cpu.main.io",
            16,
            common::ByteOrder::Little,
        ));
        spaces
    }

    fn read_register(&self, processor: &str, register: &str) -> Result<u128, common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::z80_read(&self.main_cpu, register),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn write_register(
        &mut self,
        processor: &str,
        register: &str,
        value: u128,
    ) -> Result<(), common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::z80_write(&mut self.main_cpu, register, value),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn protected_mode_state(
        &self,
        processor: &str,
    ) -> Result<common::ProtectedModeState, common::InspectError> {
        match processor {
            "cpu.main" => Err(common::InspectError::Unsupported),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn peek_memory(
        &mut self,
        space: &str,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<(), common::InspectError> {
        match space {
            "cpu.main.memory" => {
                for (index, byte) in buffer.iter_mut().enumerate() {
                    *byte = self
                        .bus
                        .peek_byte(common::inspect::offset_u16(address, index)?);
                }
                Ok(())
            }
            "cpu.main.io" => Err(common::InspectError::NotPeekable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }

    fn poke_memory(
        &mut self,
        space: &str,
        address: u64,
        bytes: &[u8],
    ) -> Result<(), common::InspectError> {
        match space {
            "cpu.main.memory" => {
                for (index, byte) in bytes.iter().enumerate() {
                    self.bus
                        .poke_byte(common::inspect::offset_u16(address, index)?, *byte);
                }
                Ok(())
            }
            "cpu.main.io" => Err(common::InspectError::NotWritable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

impl<T: TraceSink> X1Machine<T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(main_cpu: cpu::Z80, bus: X1Bus<T>) -> Self {
        Self { main_cpu, bus }
    }

    /// Runs the main CPU for up to `budget` main-clock cycles, returning the
    /// cycles actually advanced. Execution is sliced to the next scheduled event
    /// so periodic interrupts fire promptly.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target_cycle = start_cycle.saturating_add(budget);

        // Bound continuous-mode DMA stalls to this run so a long floppy transfer
        // is sliced across audio steps rather than overrunning one in a single
        // instruction.
        self.bus.set_dma_stall_deadline(target_cycle);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let slice_end = self
                .bus
                .scheduler
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

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = X1RuntimeState {
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
                Ok(X1RuntimeState {
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

impl<T: TraceSink> common::Machine for X1Machine<T> {
    fn capture_state(&mut self) -> Result<common::MachineStateBlob, common::SaveStateError> {
        self.capture_machine_blob()
    }

    fn restore_state(
        &mut self,
        blob: &common::MachineStateBlob,
    ) -> Result<(), common::SaveStateError> {
        self.restore_machine_blob(blob)
    }

    fn set_host_date_time_source(&mut self, source: common::SharedHostDateTimeSource) {
        self.bus.set_host_date_time_source(source);
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
        X1Machine::run_for(self, budget)
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

    fn translate_host_key(&self, key: HostKey, _modifiers: KeyModifiers) -> Option<u8> {
        Some(match key {
            HostKey::A => 0x41,
            HostKey::B => 0x42,
            HostKey::C => 0x43,
            HostKey::D => 0x44,
            HostKey::E => 0x45,
            HostKey::F => 0x46,
            HostKey::G => 0x47,
            HostKey::H => 0x48,
            HostKey::I => 0x49,
            HostKey::J => 0x4A,
            HostKey::K => 0x4B,
            HostKey::L => 0x4C,
            HostKey::M => 0x4D,
            HostKey::N => 0x4E,
            HostKey::O => 0x4F,
            HostKey::P => 0x50,
            HostKey::Q => 0x51,
            HostKey::R => 0x52,
            HostKey::S => 0x53,
            HostKey::T => 0x54,
            HostKey::U => 0x55,
            HostKey::V => 0x56,
            HostKey::W => 0x57,
            HostKey::X => 0x58,
            HostKey::Y => 0x59,
            HostKey::Z => 0x5A,
            HostKey::Digit0 => 0x30,
            HostKey::Digit1 => 0x31,
            HostKey::Digit2 => 0x32,
            HostKey::Digit3 => 0x33,
            HostKey::Digit4 => 0x34,
            HostKey::Digit5 => 0x35,
            HostKey::Digit6 => 0x36,
            HostKey::Digit7 => 0x37,
            HostKey::Digit8 => 0x38,
            HostKey::Digit9 => 0x39,
            HostKey::Space => 0x20,
            HostKey::Return => 0x0D,
            HostKey::KpEnter => 0x0D,
            HostKey::Backspace => 0x08,
            HostKey::Delete => 0x2E,
            HostKey::Insert => 0x2D,
            HostKey::Tab => 0x09,
            HostKey::Escape => 0x1B,
            HostKey::Home => 0x24,
            HostKey::End => 0x23,
            HostKey::PageUp => 0x21,
            HostKey::PageDown => 0x22,
            HostKey::Left => 0x25,
            HostKey::Up => 0x26,
            HostKey::Right => 0x27,
            HostKey::Down => 0x28,
            HostKey::Kp0 => 0x60,
            HostKey::Kp1 => 0x61,
            HostKey::Kp2 => 0x62,
            HostKey::Kp3 => 0x63,
            HostKey::Kp4 => 0x64,
            HostKey::Kp5 => 0x65,
            HostKey::Kp6 => 0x66,
            HostKey::Kp7 => 0x67,
            HostKey::Kp8 => 0x68,
            HostKey::Kp9 => 0x69,
            HostKey::KpMultiply => 0x6A,
            HostKey::KpPlus => 0x6B,
            HostKey::KpComma => 0x6C,
            HostKey::KpMinus => 0x6D,
            HostKey::KpPeriod => 0x6E,
            HostKey::KpDivide => 0x6F,
            HostKey::Semicolon => 0x02,
            HostKey::Apostrophe => 0x01,
            HostKey::Comma => 0x04,
            HostKey::Period => 0x06,
            HostKey::Slash => 0x07,
            HostKey::Minus => 0x05,
            HostKey::Grave => 0x0A,
            HostKey::LeftBracket => 0x0B,
            HostKey::RightBracket => 0x0E,
            HostKey::Backslash => 0x0C,
            HostKey::Equals => 0x0F,
            HostKey::LeftShift => 0x10,
            HostKey::RightShift => 0x10,
            HostKey::LeftControl => 0x11,
            HostKey::RightControl => 0x11,
            HostKey::LeftAlt => 0x12,
            HostKey::RightAlt => 0x12,
            HostKey::CapsLock => 0x14,
            _ => return None,
        })
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

    fn insert_floppy(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        let parsed =
            device::floppy::load_floppy_image(std::path::Path::new(image.name), image.bytes)
                .map_err(|error| format!("{}: {error}", image.name))?;
        let description = format!("{} ({})", parsed.name, parsed.format_name());
        self.bus.insert_floppy_backed(drive, parsed, backing);
        Ok(description)
    }

    fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.floppy_image_bytes(drive)
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
    use common::{Machine, TraceAccessKind, TraceEvent, TraceSink};

    use super::X1Machine;
    use crate::{bus::X1Bus, config::X1Model, rom::LoadedRoms};

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
        let model = X1Model::X1;
        let roms = LoadedRoms {
            model,
            ipl: vec![0; model.ipl_rom_size()],
            cgrom_8x8: vec![0; 0x0800],
            ank: vec![0; 0x2000],
            kanji: None,
        };
        let mut bus = X1Bus::new_with_trace_sink(model, 48_000, YieldOnScheduled::default());
        bus.load_roms(&roms);
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = X1Machine::new(main_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn both_models_replay_from_a_runtime_state() {
        for model in [X1Model::X1, X1Model::X1Turbo] {
            let roms = LoadedRoms {
                model,
                ipl: vec![0; model.ipl_rom_size()],
                cgrom_8x8: vec![0; 0x0800],
                ank: vec![0; 0x2000],
                kanji: model.is_turbo().then(|| vec![0; 0x20000]),
            };
            let mut bus = X1Bus::new(model, 48_000);
            bus.load_roms(&roms);
            let mut tape = 4_000u32.to_le_bytes().to_vec();
            tape.extend_from_slice(&[0xAA; 64]);
            bus.insert_cassette("tap", &tape).unwrap();
            let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
            let mut machine = X1Machine::new(main_cpu, bus);
            for (register, value) in [(0x00, 0x40), (0x01, 0x00), (0x08, 0x0F), (0x07, 0x3E)] {
                machine.bus.io_write(0x1C00, register);
                machine.bus.io_write(0x1B00, value);
            }
            machine.bus.io_write(0x1900, 0xE9);
            machine.run_for(8_000);
            machine.bus.io_write(0x1900, 0x02);
            machine.run_for(8_000);
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
        let model = X1Model::X1Turbo;
        let roms = LoadedRoms {
            model,
            ipl: vec![0; model.ipl_rom_size()],
            cgrom_8x8: vec![0; 0x0800],
            ank: vec![0; 0x2000],
            kanji: Some(vec![0; 0x20000]),
        };
        let mut bus = X1Bus::new(model, 48_000);
        bus.load_roms(&roms);
        let mut machine = X1Machine::new(cpu::Z80::new(bus.cpu_clock_hz()), bus);
        machine.run_for(10_000);
        let valid = machine.capture_state().unwrap();
        let before = valid.payload().to_vec();
        let corrupt = valid
            .with_payload(valid.payload()[..valid.payload().len() / 2].to_vec())
            .unwrap();

        assert!(machine.restore_state(&corrupt).is_err());
        assert_eq!(machine.capture_state().unwrap().payload(), before);
    }
}
