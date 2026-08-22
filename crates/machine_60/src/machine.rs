//! Top-level PC-6000 machine.
//!
//! A single-Z80 machine: one CPU driving one bus, paced by a monotonic
//! `current_cycle` in main-clock units.

use common::{CpuZ80, HostKey, KeyModifiers, NoTrace, TraceSink};

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

/// Builds an automated PC-6000 machine around a configured bus.
pub fn build_automated_machine(
    bus: Pc6000Bus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(Pc6000Machine::new(main_cpu, bus))
}

impl common::AutomationDriver for Pc6000Machine<common::tracing::ApplicationTraceSink> {
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
        Pc6000Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for Pc6000Machine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "pc60",
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
        PC60_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the PC-6000 bus.
const PC60_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::PC60_IRQ],
    scheduled: common::trace_id::scheduled::PC60,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::PC60_FDC,
        actions: &[common::trace_action(common::trace_id::action::READ)],
    }],
    providers: &[],
};

impl common::MachineInspector for Pc6000Machine<common::tracing::ApplicationTraceSink> {
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

    /// Maps a host key plus modifier state to its PC-6001 firmware keycode.
    ///
    /// Letters carry their uppercase ASCII code, digits and punctuation their
    /// ASCII code, and the function keys F1-F5 the wire ids 0x60-0x64. The
    /// PC-6001 sub-controller expects a pre-composed code, so Control folds a
    /// letter to its control code (0x01-0x1A) and Shift selects the shifted code.
    /// Control takes precedence over Shift, matching the real key scan.
    fn translate_host_key(&self, key: HostKey, modifiers: KeyModifiers) -> Option<u8> {
        const PC60_FUNCTION_KEY_BASE: u8 = 0x60;
        let base = match key {
            HostKey::A => b'A',
            HostKey::B => b'B',
            HostKey::C => b'C',
            HostKey::D => b'D',
            HostKey::E => b'E',
            HostKey::F => b'F',
            HostKey::G => b'G',
            HostKey::H => b'H',
            HostKey::I => b'I',
            HostKey::J => b'J',
            HostKey::K => b'K',
            HostKey::L => b'L',
            HostKey::M => b'M',
            HostKey::N => b'N',
            HostKey::O => b'O',
            HostKey::P => b'P',
            HostKey::Q => b'Q',
            HostKey::R => b'R',
            HostKey::S => b'S',
            HostKey::T => b'T',
            HostKey::U => b'U',
            HostKey::V => b'V',
            HostKey::W => b'W',
            HostKey::X => b'X',
            HostKey::Y => b'Y',
            HostKey::Z => b'Z',
            HostKey::Digit0 => b'0',
            HostKey::Digit1 => b'1',
            HostKey::Digit2 => b'2',
            HostKey::Digit3 => b'3',
            HostKey::Digit4 => b'4',
            HostKey::Digit5 => b'5',
            HostKey::Digit6 => b'6',
            HostKey::Digit7 => b'7',
            HostKey::Digit8 => b'8',
            HostKey::Digit9 => b'9',
            HostKey::Space => b' ',
            HostKey::Minus => b'-',
            HostKey::Comma => b',',
            HostKey::Period => b'.',
            HostKey::Slash => b'/',
            HostKey::Semicolon => b';',
            HostKey::LeftBracket => b'[',
            HostKey::RightBracket => b']',
            HostKey::Equals => b'^',
            HostKey::Return | HostKey::KpEnter => 0x0D,
            HostKey::Backspace | HostKey::Delete => 0x08,
            HostKey::Tab => 0x09,
            HostKey::Right => 0x1C,
            HostKey::Left => 0x1D,
            HostKey::Up => 0x1E,
            HostKey::Down => 0x1F,
            HostKey::F1 => PC60_FUNCTION_KEY_BASE,
            HostKey::F2 => PC60_FUNCTION_KEY_BASE + 1,
            HostKey::F3 => PC60_FUNCTION_KEY_BASE + 2,
            HostKey::F4 => PC60_FUNCTION_KEY_BASE + 3,
            HostKey::F5 => PC60_FUNCTION_KEY_BASE + 4,
            _ => return None,
        };
        if modifiers.ctrl && (0x40..=0x5F).contains(&base) {
            return Some(base & 0x1F);
        }
        if modifiers.shift {
            return Some(match key {
                HostKey::A => b'a',
                HostKey::B => b'b',
                HostKey::C => b'c',
                HostKey::D => b'd',
                HostKey::E => b'e',
                HostKey::F => b'f',
                HostKey::G => b'g',
                HostKey::H => b'h',
                HostKey::I => b'i',
                HostKey::J => b'j',
                HostKey::K => b'k',
                HostKey::L => b'l',
                HostKey::M => b'm',
                HostKey::N => b'n',
                HostKey::O => b'o',
                HostKey::P => b'p',
                HostKey::Q => b'q',
                HostKey::R => b'r',
                HostKey::S => b's',
                HostKey::T => b't',
                HostKey::U => b'u',
                HostKey::V => b'v',
                HostKey::W => b'w',
                HostKey::X => b'x',
                HostKey::Y => b'y',
                HostKey::Z => b'z',
                HostKey::Digit1 => b'!',
                HostKey::Digit2 => b'"',
                HostKey::Digit3 => b'#',
                HostKey::Digit4 => b'$',
                HostKey::Digit5 => b'%',
                HostKey::Digit6 => b'&',
                HostKey::Digit7 => b'\'',
                HostKey::Digit8 => b'(',
                HostKey::Digit9 => b')',
                HostKey::Digit0 => b'=',
                HostKey::Comma => b';',
                HostKey::Period => b':',
                HostKey::Slash => b'?',
                HostKey::F1 => PC60_FUNCTION_KEY_BASE + 5,
                HostKey::F2 => PC60_FUNCTION_KEY_BASE + 6,
                HostKey::F3 => PC60_FUNCTION_KEY_BASE + 7,
                HostKey::F4 => PC60_FUNCTION_KEY_BASE + 8,
                HostKey::F5 => PC60_FUNCTION_KEY_BASE + 9,
                _ => base,
            });
        }
        Some(base)
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
    fn translate_host_key_folds_shift_and_control() {
        let bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        let machine = Pc6000Machine::new(cpu::Z80::new(bus.cpu_clock_hz()), bus);
        let none = KeyModifiers::default();
        let shift = KeyModifiers {
            shift: true,
            ctrl: false,
        };
        let control = KeyModifiers {
            shift: false,
            ctrl: true,
        };
        let both = KeyModifiers {
            shift: true,
            ctrl: true,
        };

        // Letters carry uppercase ASCII; Shift lowercases and Control folds to a
        // control code, with Control taking precedence over Shift.
        assert_eq!(machine.translate_host_key(HostKey::A, none), Some(b'A'));
        assert_eq!(machine.translate_host_key(HostKey::A, shift), Some(b'a'));
        assert_eq!(machine.translate_host_key(HostKey::A, control), Some(0x01));
        assert_eq!(machine.translate_host_key(HostKey::C, control), Some(0x03));
        assert_eq!(machine.translate_host_key(HostKey::A, both), Some(0x01));

        // Shifted number row and punctuation carry their symbols.
        assert_eq!(
            machine.translate_host_key(HostKey::Digit1, shift),
            Some(b'!')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Digit2, shift),
            Some(b'"')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Digit7, shift),
            Some(b'\'')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Digit0, shift),
            Some(b'=')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Comma, shift),
            Some(b';')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Period, shift),
            Some(b':')
        );
        assert_eq!(
            machine.translate_host_key(HostKey::Slash, shift),
            Some(b'?')
        );

        // Shifted F1 carries the upper wire id; the base is the lower id.
        assert_eq!(machine.translate_host_key(HostKey::F1, none), Some(0x60));
        assert_eq!(machine.translate_host_key(HostKey::F1, shift), Some(0x65));

        // The physical '=' key resolves to the PC-6001 caret code.
        assert_eq!(
            machine.translate_host_key(HostKey::Equals, none),
            Some(b'^')
        );

        // Bare modifiers stay no-key.
        assert_eq!(machine.translate_host_key(HostKey::LeftShift, none), None);
        assert_eq!(machine.translate_host_key(HostKey::LeftControl, none), None);
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
            .as_chunks::<4>()
            .0
            .iter()
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
