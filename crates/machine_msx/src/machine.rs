//! Top-level MSX machine.
//!
//! One Z80 drives one bus paced by a monotonic cycle in normal CPU T-states.

use common::{CpuZ80, HostKey, KeyModifiers, NoTrace, TraceSink};

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

/// Builds an automated machine around a configured bus.
pub fn build_automated_machine(
    bus: MsxBus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    Box::new(MsxMachine::new(main_cpu, bus))
}

impl common::AutomationDriver for MsxMachine<common::tracing::ApplicationTraceSink> {
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
        MsxMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for MsxMachine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "msx",
            model: self.bus.model_id(),
            timebase: common::AutomationTimebase {
                ticks_per_second_numerator: numerator,
                ticks_per_second_denominator: denominator,
            },
            audio_sample_rate: self.bus.audio_sample_rate(),
            input: common::InputCapabilities {
                keyboard: true,
                mouse_buttons: 2,
                joystick_ports: 2,
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
        MSX_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the MSX bus.
const MSX_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::MSX_IRQ],
    scheduled: common::trace_id::scheduled::MSX,
    devices: &[
        common::TraceDeviceCatalog {
            device: common::trace_id::device::MSX_SLOT,
            actions: &[common::trace_action(common::trace_id::action::SELECT)],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::MSX_MAPPER,
            actions: &[common::trace_action(common::trace_id::action::BANK)],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::MSX_FDC,
            actions: &[common::trace_action(common::trace_id::action::READ)],
        },
    ],
    providers: &[],
};

impl common::MachineInspector for MsxMachine<common::tracing::ApplicationTraceSink> {
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

    fn set_host_date_time_source(&mut self, source: common::SharedHostDateTimeSource) {
        self.bus.set_host_date_time_source(source);
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

    fn translate_host_key(&self, key: HostKey, _modifiers: KeyModifiers) -> Option<u8> {
        Some(match key {
            HostKey::Escape => 0x00,
            HostKey::Digit1 => 0x01,
            HostKey::Digit2 => 0x02,
            HostKey::Digit3 => 0x03,
            HostKey::Digit4 => 0x04,
            HostKey::Digit5 => 0x05,
            HostKey::Digit6 => 0x06,
            HostKey::Digit7 => 0x07,
            HostKey::Digit8 => 0x08,
            HostKey::Digit9 => 0x09,
            HostKey::Digit0 => 0x0A,
            HostKey::Minus => 0x0B,
            HostKey::Equals => 0x0C,
            HostKey::Backslash => 0x0D,
            HostKey::Backspace => 0x0E,
            HostKey::Tab => 0x0F,
            HostKey::Q => 0x10,
            HostKey::W => 0x11,
            HostKey::E => 0x12,
            HostKey::R => 0x13,
            HostKey::T => 0x14,
            HostKey::Y => 0x15,
            HostKey::U => 0x16,
            HostKey::I => 0x17,
            HostKey::O => 0x18,
            HostKey::P => 0x19,
            HostKey::Grave => 0x1A,
            HostKey::LeftBracket => 0x1B,
            HostKey::Return => 0x1C,
            HostKey::A => 0x1D,
            HostKey::S => 0x1E,
            HostKey::D => 0x1F,
            HostKey::F => 0x20,
            HostKey::G => 0x21,
            HostKey::H => 0x22,
            HostKey::J => 0x23,
            HostKey::K => 0x24,
            HostKey::L => 0x25,
            HostKey::Semicolon => 0x26,
            HostKey::Apostrophe => 0x27,
            HostKey::RightBracket => 0x28,
            HostKey::Z => 0x29,
            HostKey::X => 0x2A,
            HostKey::C => 0x2B,
            HostKey::V => 0x2C,
            HostKey::B => 0x2D,
            HostKey::N => 0x2E,
            HostKey::M => 0x2F,
            HostKey::Comma => 0x30,
            HostKey::Period => 0x31,
            HostKey::Slash => 0x32,
            HostKey::NonUsBackslash => 0x33,
            HostKey::Space => 0x34,
            HostKey::RightAlt => 0x35,
            HostKey::PageUp => 0x36,
            HostKey::PageDown => 0x37,
            HostKey::Insert => 0x38,
            HostKey::Delete => 0x39,
            HostKey::Up => 0x3A,
            HostKey::Left => 0x3B,
            HostKey::Right => 0x3C,
            HostKey::Down => 0x3D,
            HostKey::Home => 0x3E,
            HostKey::End => 0x3F,
            HostKey::KpMinus => 0x40,
            HostKey::KpDivide => 0x41,
            HostKey::Kp7 => 0x42,
            HostKey::Kp8 => 0x43,
            HostKey::Kp9 => 0x44,
            HostKey::KpMultiply => 0x45,
            HostKey::Kp4 => 0x46,
            HostKey::Kp5 => 0x47,
            HostKey::Kp6 => 0x48,
            HostKey::KpPlus => 0x49,
            HostKey::Kp1 => 0x4A,
            HostKey::Kp2 => 0x4B,
            HostKey::Kp3 => 0x4C,
            HostKey::KpEnter => 0x4D,
            HostKey::Kp0 => 0x4E,
            HostKey::KpComma => 0x4F,
            HostKey::KpPeriod => 0x50,
            HostKey::Application => 0x51,
            HostKey::F11 => 0x52,
            HostKey::F12 => 0x53,
            HostKey::F13 => 0x54,
            HostKey::F14 => 0x55,
            HostKey::F15 => 0x56,
            HostKey::Pause => 0x60,
            HostKey::PrintScreen => 0x61,
            HostKey::F1 => 0x62,
            HostKey::F2 => 0x63,
            HostKey::F3 => 0x64,
            HostKey::F4 => 0x65,
            HostKey::F5 => 0x66,
            HostKey::F6 => 0x67,
            HostKey::F7 => 0x68,
            HostKey::F8 => 0x69,
            HostKey::F9 => 0x6A,
            HostKey::F10 => 0x6B,
            HostKey::LeftShift => 0x70,
            HostKey::RightShift => 0x70,
            HostKey::CapsLock => 0x71,
            HostKey::NumLock => 0x72,
            HostKey::LeftAlt => 0x73,
            HostKey::LeftControl => 0x74,
            HostKey::International1 => 0x33,
            HostKey::International2 => 0x72,
            HostKey::International3 => 0x0D,
            HostKey::International4 => 0x35,
            HostKey::International5 => 0x51,
            _ => return None,
        })
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

    fn insert_floppy(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        if drive >= usize::from(self.bus.model().drive_count()) {
            return Err(format!("MSX drive {drive} is not present"));
        }
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
        for (bank, bytes) in image.as_chunks_mut::<0x2000>().0.iter_mut().enumerate() {
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
