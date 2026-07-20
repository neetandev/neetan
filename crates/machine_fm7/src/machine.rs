//! FM-7 machine.
//!
//! The main and display MC6809 CPUs share the machine bus and are interleaved
//! according to their respective clock rates.

use common::{Cpu6809, HostKey, KeyModifiers, NoTrace, TraceSink};

use crate::bus::{Fm7Bus, MainBusView, SubBusView};

save_state::runtime_state! {
/// Machine-root state for one FM-7 family snapshot.
#[derive(Clone)]
struct Fm7RuntimeState {
    main_cpu: cpu::M6809State,
    sub_cpu: cpu::M6809State,
    bus: crate::bus::Fm7BusState,
    sub_cycle_target: u64,
}}

/// Main-clock cycles per interleave slice during normal execution.
const DEFAULT_SLICE_CYCLES: u64 = 16;
/// Main-clock cycles per interleave slice while a main/sub handshake is active.
const HANDSHAKE_SLICE_CYCLES: u64 = 4;

/// FM-7 / FM-77AV machine.
pub struct Fm7Machine<T: TraceSink = NoTrace> {
    /// Main CPU.
    pub main_cpu: cpu::M6809,
    /// Display sub CPU.
    pub sub_cpu: cpu::M6809,
    /// System bus.
    pub bus: Fm7Bus<T>,
    /// Sub CPU cycle target, tracking the cycles it owes so whole-instruction
    /// overshoot in one slice is absorbed by the next.
    sub_cycle_target: u64,
}

/// Builds an untraced FM-7 machine around a configured bus.
pub fn build_untraced_machine(
    model: crate::Fm7Model,
    bus: Fm7Bus<NoTrace>,
) -> Box<dyn common::Machine> {
    let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
    Box::new(Fm7Machine::new(main_cpu, sub_cpu, bus))
}

/// Builds an automated FM-7 machine around a configured bus.
pub fn build_automated_machine(
    model: crate::Fm7Model,
    bus: Fm7Bus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
    Box::new(Fm7Machine::new(main_cpu, sub_cpu, bus))
}

impl common::AutomationDriver for Fm7Machine<common::tracing::ApplicationTraceSink> {
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
        Fm7Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for Fm7Machine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "fm7",
            model: self.bus.model_id(),
            timebase: common::AutomationTimebase {
                ticks_per_second_numerator: numerator,
                ticks_per_second_denominator: denominator,
            },
            audio_sample_rate: self.bus.audio_sample_rate(),
            input: common::InputCapabilities {
                keyboard: true,
                mouse_buttons: 2,
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
        FM7_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the FM-7 bus.
const FM7_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[
        common::trace_id::controller::FM7_MAIN_IRQ,
        common::trace_id::controller::FM7_MAIN_FIRQ,
        common::trace_id::controller::FM7_SUB_IRQ,
        common::trace_id::controller::FM7_SUB_FIRQ,
        common::trace_id::controller::FM7_SUB_NMI,
    ],
    scheduled: common::trace_id::scheduled::FM7,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::FM7_FDC,
        actions: &[common::trace_id::action::READ],
    }],
    providers: &[],
};

impl common::MachineInspector for Fm7Machine<common::tracing::ApplicationTraceSink> {
    fn processors(&self) -> common::ProcessorList {
        let mut processors = common::ProcessorList::new();
        processors.push(common::inspect::m6809_processor("cpu.main"));
        processors.push(common::inspect::m6809_processor("cpu.sub"));
        processors
    }

    fn address_spaces(&self) -> common::AddressSpaceList {
        // The MC6809 is memory-mapped only; there is no separate I/O space.
        let mut spaces = common::AddressSpaceList::new();
        spaces.push(common::inspect::memory_space(
            "cpu.main.memory",
            16,
            common::ByteOrder::Big,
        ));
        spaces.push(common::inspect::memory_space(
            "cpu.sub.memory",
            16,
            common::ByteOrder::Big,
        ));
        spaces
    }

    fn read_register(&self, processor: &str, register: &str) -> Result<u128, common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::m6809_read(&self.main_cpu, register),
            "cpu.sub" => common::inspect::m6809_read(&self.sub_cpu, register),
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
            "cpu.main" => common::inspect::m6809_write(&mut self.main_cpu, register, value),
            "cpu.sub" => common::inspect::m6809_write(&mut self.sub_cpu, register, value),
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn protected_mode_state(
        &self,
        processor: &str,
    ) -> Result<common::ProtectedModeState, common::InspectError> {
        match processor {
            "cpu.main" | "cpu.sub" => Err(common::InspectError::Unsupported),
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
            "cpu.sub.memory" => {
                for (index, byte) in buffer.iter_mut().enumerate() {
                    *byte = self
                        .bus
                        .peek_sub_byte(common::inspect::offset_u16(address, index)?);
                }
                Ok(())
            }
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
            "cpu.sub.memory" => {
                for (index, byte) in bytes.iter().enumerate() {
                    self.bus
                        .poke_sub_byte(common::inspect::offset_u16(address, index)?, *byte);
                }
                Ok(())
            }
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

impl<T: TraceSink> Fm7Machine<T> {
    /// Creates a new machine from the given CPUs and bus.
    pub fn new(mut main_cpu: cpu::M6809, mut sub_cpu: cpu::M6809, mut bus: Fm7Bus<T>) -> Self {
        {
            let mut view = MainBusView { bus: &mut bus };
            main_cpu.reset_with_bus(&mut view);
        }
        {
            let mut view = SubBusView { bus: &mut bus };
            sub_cpu.reset_with_bus(&mut view);
        }
        let sub_cycle_target = bus.sub_cycle();
        Self {
            main_cpu,
            sub_cpu,
            bus,
            sub_cycle_target,
        }
    }

    /// Resets the main CPU and fetches its reset vector from the bus.
    pub fn reset_main_cpu(&mut self) {
        let mut view = MainBusView { bus: &mut self.bus };
        self.main_cpu.reset_with_bus(&mut view);
    }

    /// Resets the sub CPU and fetches its reset vector from the sub-monitor ROM.
    pub fn reset_sub_cpu(&mut self) {
        let mut view = SubBusView { bus: &mut self.bus };
        self.sub_cpu.reset_with_bus(&mut view);
    }

    /// Runs the main CPU for up to `budget` main-clock cycles.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        if T::ENABLED {
            if self.bus.take_clock_reanchor() {
                self.sub_cycle_target = self.bus.sub_cycle();
            }
            if self.bus.take_sub_reset() {
                self.reset_sub_cpu();
            }
            self.run_sub_to_target();
            if self.bus.tracer().yield_requested() {
                return 0;
            }
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let next_event_cycle = self
                .bus
                .next_event_cycle()
                .unwrap_or(target_cycle)
                .min(target_cycle);
            let slice_cap = if self.bus.handshake_active() {
                HANDSHAKE_SLICE_CYCLES
            } else {
                DEFAULT_SLICE_CYCLES
            };
            let slice_end = current_cycle
                .saturating_add(slice_cap)
                .min(next_event_cycle);

            self.sync_main_firq();
            let slice_cycles = slice_end.saturating_sub(current_cycle).max(1);
            let ran_cycles = {
                let mut view = MainBusView { bus: &mut self.bus };
                self.main_cpu.run_for(slice_cycles, &mut view)
            };
            let trace_yield_requested = T::ENABLED && self.bus.tracer().yield_requested();
            if !trace_yield_requested && ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }

            if self.bus.take_clock_reanchor() {
                self.sub_cycle_target = self.bus.sub_cycle();
            }

            let elapsed_cycles = self.bus.current_cycle().saturating_sub(current_cycle);
            self.account_sub_for_main_cycles(elapsed_cycles);
            if trace_yield_requested {
                break;
            }
            if self.bus.take_sub_reset() {
                self.reset_sub_cpu();
            }
            self.run_sub_to_target();
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }

            if self.bus.current_cycle() >= next_event_cycle {
                self.bus.process_events();
                if T::ENABLED && self.bus.tracer().yield_requested() {
                    break;
                }
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    /// Mirrors the bus FIRQ level into the main CPU's internal latch.
    fn sync_main_firq(&mut self) {
        if self.bus.firq_active() {
            self.main_cpu.request_firq();
        } else {
            self.main_cpu.clear_firq();
        }
    }

    /// Mirrors the sub CPU keyboard FIRQ level into the sub CPU's internal latch.
    fn sync_sub_firq(&mut self) {
        if self.bus.sub_has_firq() {
            self.sub_cpu.request_firq();
        } else {
            self.sub_cpu.clear_firq();
        }
    }

    /// Adds the sub CPU cycles owed for elapsed main-clock time.
    fn account_sub_for_main_cycles(&mut self, main_cycles: u64) {
        let owed_cycles = self.bus.sub_cycles_for_main_units(main_cycles);
        self.sub_cycle_target = self.sub_cycle_target.saturating_add(owed_cycles);
    }

    /// Runs or idles the sub CPU to its accumulated cycle target.
    fn run_sub_to_target(&mut self) {
        let sub_now = self.bus.sub_cycle();

        if self.bus.sub_halt_active() {
            self.bus.set_sub_halted(true);
            self.bus
                .advance_sub_cycle(self.sub_cycle_target.saturating_sub(sub_now));
            return;
        }

        self.bus.set_sub_halted(false);
        let budget = self.sub_cycle_target.saturating_sub(sub_now);
        if budget == 0 {
            return;
        }
        self.sync_sub_firq();
        let mut view = SubBusView { bus: &mut self.bus };
        self.sub_cpu.run_for(budget, &mut view);
    }

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = Fm7RuntimeState {
            main_cpu: self.main_cpu.capture_state(),
            sub_cpu: self.sub_cpu.capture_state(),
            bus: self.bus.capture_runtime_state()?,
            sub_cycle_target: self.sub_cycle_target,
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
                Ok(Fm7RuntimeState {
                    main_cpu: machine.main_cpu.capture_state(),
                    sub_cpu: machine.sub_cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                    sub_cycle_target: machine.sub_cycle_target,
                })
            },
            |machine, state| {
                machine.main_cpu.restore_state(state.main_cpu)?;
                machine.sub_cpu.restore_state(state.sub_cpu)?;
                machine.bus.restore_runtime_state(state.bus)?;
                machine.sub_cycle_target = state.sub_cycle_target;
                Ok(())
            },
        )
    }
}

impl<T: TraceSink> common::Machine for Fm7Machine<T> {
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
        Fm7Machine::run_for(self, budget)
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
            HostKey::Escape => 0x01,
            HostKey::Digit1 => 0x02,
            HostKey::Digit2 => 0x03,
            HostKey::Digit3 => 0x04,
            HostKey::Digit4 => 0x05,
            HostKey::Digit5 => 0x06,
            HostKey::Digit6 => 0x07,
            HostKey::Digit7 => 0x08,
            HostKey::Digit8 => 0x09,
            HostKey::Digit9 => 0x0A,
            HostKey::Digit0 => 0x0B,
            HostKey::Minus => 0x0C,
            HostKey::Equals => 0x0D,
            HostKey::Backslash => 0x0E,
            HostKey::Backspace => 0x0F,
            HostKey::Tab => 0x10,
            HostKey::Q => 0x11,
            HostKey::W => 0x12,
            HostKey::E => 0x13,
            HostKey::R => 0x14,
            HostKey::T => 0x15,
            HostKey::Y => 0x16,
            HostKey::U => 0x17,
            HostKey::I => 0x18,
            HostKey::O => 0x19,
            HostKey::P => 0x1A,
            HostKey::Grave => 0x1B,
            HostKey::LeftBracket => 0x1C,
            HostKey::Return => 0x1D,
            HostKey::A => 0x1E,
            HostKey::S => 0x1F,
            HostKey::D => 0x20,
            HostKey::F => 0x21,
            HostKey::G => 0x22,
            HostKey::H => 0x23,
            HostKey::J => 0x24,
            HostKey::K => 0x25,
            HostKey::L => 0x26,
            HostKey::Semicolon => 0x27,
            HostKey::Apostrophe => 0x28,
            HostKey::RightBracket => 0x29,
            HostKey::Z => 0x2A,
            HostKey::X => 0x2B,
            HostKey::C => 0x2C,
            HostKey::V => 0x2D,
            HostKey::B => 0x2E,
            HostKey::N => 0x2F,
            HostKey::M => 0x30,
            HostKey::Comma => 0x31,
            HostKey::Period => 0x32,
            HostKey::Slash => 0x33,
            HostKey::NonUsBackslash => 0x34,
            HostKey::Space => 0x35,
            HostKey::KpMultiply => 0x36,
            HostKey::KpDivide => 0x37,
            HostKey::KpPlus => 0x38,
            HostKey::KpMinus => 0x39,
            HostKey::Kp7 => 0x3A,
            HostKey::Kp8 => 0x3B,
            HostKey::Kp9 => 0x3C,
            HostKey::Kp4 => 0x3E,
            HostKey::Kp5 => 0x3F,
            HostKey::Kp6 => 0x40,
            HostKey::KpComma => 0x41,
            HostKey::Kp1 => 0x42,
            HostKey::Kp2 => 0x43,
            HostKey::Kp3 => 0x44,
            HostKey::KpEnter => 0x45,
            HostKey::Kp0 => 0x46,
            HostKey::KpPeriod => 0x47,
            HostKey::Home => 0x49,
            HostKey::Delete => 0x4B,
            HostKey::Insert => 0x4C,
            HostKey::Up => 0x4D,
            HostKey::Left => 0x4F,
            HostKey::Down => 0x50,
            HostKey::Right => 0x51,
            HostKey::LeftControl => 0x52,
            HostKey::RightControl => 0x52,
            HostKey::LeftShift => 0x53,
            HostKey::RightShift => 0x54,
            HostKey::CapsLock => 0x55,
            HostKey::LeftAlt => 0x56,
            HostKey::RightAlt => 0x56,
            HostKey::Pause => 0x5C,
            HostKey::F1 => 0x5D,
            HostKey::F2 => 0x5E,
            HostKey::F3 => 0x5F,
            HostKey::F4 => 0x60,
            HostKey::F5 => 0x61,
            HostKey::F6 => 0x62,
            HostKey::F7 => 0x63,
            HostKey::F8 => 0x64,
            HostKey::F9 => 0x65,
            HostKey::F10 => 0x66,
            _ => return None,
        })
    }

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        if index == 0 {
            self.bus.set_joystick(state);
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
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use super::Fm7Machine;
    use crate::{
        bus::Fm7Bus,
        config::{BootMode, Fm7Model},
        rom::LoadedRoms,
    };

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
    struct YieldOnMainFetch {
        armed: bool,
        yield_requested: bool,
    }

    impl YieldOnMainFetch {
        /// Arms a one-shot yield on the next main-CPU fetch.
        fn arm(&mut self) {
            self.armed = true;
        }

        /// Clears the one-shot yield request.
        fn resume(&mut self) {
            self.armed = false;
            self.yield_requested = false;
        }
    }

    impl TraceSink for YieldOnMainFetch {
        fn trace(&mut self, context: common::TraceContext, event: TraceEvent<'_>) {
            if self.armed
                && context.source == common::trace_source::CPU_MAIN
                && matches!(
                    event,
                    TraceEvent::Access(access) if access.kind == TraceAccessKind::Fetch
                )
            {
                self.yield_requested = true;
            }
        }

        fn yield_requested(&self) -> bool {
            self.yield_requested
        }
    }

    #[test]
    fn scheduled_trace_yield_prevents_a_later_fetch() {
        let model = Fm7Model::Fm7;
        let roms = LoadedRoms {
            model,
            fbasic: vec![0; 0x7C00],
            subsys_c: vec![0; 0x2800],
            kanji: None,
            boot_bas: Some(vec![0; 0x0200]),
            boot_dos: Some(vec![0; 0x0200]),
            initiate: None,
            subsys_a: None,
            subsys_b: None,
            subsyscg: None,
        };
        let mut bus = Fm7Bus::new_with_trace_sink(
            model,
            BootMode::Basic,
            48_000,
            YieldOnScheduled::default(),
        );
        bus.load_roms(&roms);
        let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
        let mut machine = Fm7Machine::new(main_cpu, sub_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn main_trace_yield_preserves_sub_cpu_cycle_target() {
        let model = Fm7Model::Fm7;
        let roms = LoadedRoms {
            model,
            fbasic: vec![0; 0x7C00],
            subsys_c: vec![0; 0x2800],
            kanji: None,
            boot_bas: Some(vec![0; 0x0200]),
            boot_dos: Some(vec![0; 0x0200]),
            initiate: None,
            subsys_a: None,
            subsys_b: None,
            subsyscg: None,
        };
        let mut bus = Fm7Bus::new_with_trace_sink(
            model,
            BootMode::Basic,
            48_000,
            YieldOnMainFetch::default(),
        );
        bus.load_roms(&roms);
        let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
        let mut machine = Fm7Machine::new(main_cpu, sub_cpu, bus);
        machine.bus.tracer_mut().arm();

        machine.run_for(100);

        let pending_target = machine.sub_cycle_target;
        assert!(pending_target > machine.bus.sub_cycle());

        machine.bus.tracer_mut().resume();
        assert_eq!(machine.run_for(0), 0);

        assert!(machine.bus.sub_cycle() >= pending_target);
    }
}
