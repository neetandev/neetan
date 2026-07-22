//! PC-88VA2 machine: a NEC V30 main CPU and a Z80 floppy sub-CPU.
//!
//! A single monotonic `current_cycle` (main-clock units) drives the scheduler;
//! the sub CPU runs proportional T-states converted by the clock ratio. The
//! slice is kept tight, and tighter still while a PPI mailbox handshake is in
//! flight, so the two cores make progress together.

use common::{Bus, Cpu, CpuZ80, HostKey, KeyModifiers, NoTrace, TraceSink};

use crate::bus::{Pc88VaBus, SYNC_SLICE, SubBusView, TIGHT_SLICE};

save_state::runtime_state! {
/// Machine-root state for one PC-88VA snapshot.
#[derive(Clone)]
struct Pc88VaRuntimeState {
    main_cpu: cpu::V30State,
    sub_cpu: cpu::Z80State,
    bus: crate::bus::Pc88VaBusState,
}}

const RESET_CS: u16 = 0xF000;
const RESET_IP: u16 = 0xFFF0;

/// A PC-88VA2 machine: the V30 main CPU, the Z80 floppy sub-CPU, and the VA bus.
pub struct Pc88VaMachine<T: TraceSink = NoTrace> {
    /// The V30 main CPU.
    pub cpu: cpu::V30,
    /// The Z80 floppy sub-CPU (PC80S31K).
    pub sub_cpu: cpu::Z80,
    /// The VA system bus, owning memory and devices.
    pub bus: Pc88VaBus<T>,
}

/// Builds an untraced PC-88VA machine around a configured bus.
pub fn build_untraced_machine(bus: Pc88VaBus<NoTrace>) -> Box<dyn common::Machine> {
    let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
    Box::new(Pc88VaMachine::new(reset_cpu(), sub_cpu, bus))
}

/// Builds an automated PC-88VA machine around a configured bus.
pub fn build_automated_machine(
    bus: Pc88VaBus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
    Box::new(Pc88VaMachine::new(reset_cpu(), sub_cpu, bus))
}

impl common::AutomationDriver for Pc88VaMachine<common::tracing::ApplicationTraceSink> {
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
        Pc88VaMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for Pc88VaMachine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "pc88va",
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
        PC88VA_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the PC-88VA bus.
const PC88VA_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[
        common::trace_id::controller::PC88VA_PIC,
        common::trace_id::controller::PC88VA_SUB_FDC,
    ],
    scheduled: common::trace_id::scheduled::PC88VA,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::PC88VA_FDC,
        actions: &[common::trace_action(common::trace_id::action::READ)],
    }],
    providers: &[],
};

impl common::MachineInspector for Pc88VaMachine<common::tracing::ApplicationTraceSink> {
    fn processors(&self) -> common::ProcessorList {
        let mut processors = common::ProcessorList::new();
        // The main uPD9002 runs a V30 instruction set; it has no protected mode.
        processors.push(common::inspect::x86_processor("cpu.main", false));
        processors.push(common::inspect::z80_processor("cpu.sub"));
        processors
    }

    fn address_spaces(&self) -> common::AddressSpaceList {
        let mut spaces = common::AddressSpaceList::new();
        spaces.push(common::inspect::memory_space(
            "cpu.main.memory",
            20,
            common::ByteOrder::Little,
        ));
        spaces.push(common::inspect::io_space(
            "cpu.main.io",
            16,
            common::ByteOrder::Little,
        ));
        spaces.push(common::inspect::memory_space(
            "cpu.sub.memory",
            16,
            common::ByteOrder::Little,
        ));
        spaces.push(common::inspect::io_space(
            "cpu.sub.io",
            16,
            common::ByteOrder::Little,
        ));
        spaces
    }

    fn read_register(&self, processor: &str, register: &str) -> Result<u128, common::InspectError> {
        match processor {
            "cpu.main" => common::inspect::x86_read(&self.cpu, register),
            "cpu.sub" => common::inspect::z80_read(&self.sub_cpu, register),
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
            "cpu.main" => common::inspect::x86_write(&mut self.cpu, register, value),
            "cpu.sub" => common::inspect::z80_write(&mut self.sub_cpu, register, value),
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
                        .peek_byte(common::inspect::offset_u32(address, index)?);
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
            "cpu.main.io" | "cpu.sub.io" => Err(common::InspectError::NotPeekable),
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
                        .poke_byte(common::inspect::offset_u32(address, index)?, *byte);
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
            "cpu.main.io" | "cpu.sub.io" => Err(common::InspectError::NotWritable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

impl<T: TraceSink> Pc88VaMachine<T> {
    /// Builds a machine around configured CPUs and bus.
    pub fn new(cpu: cpu::V30, sub_cpu: cpu::Z80, bus: Pc88VaBus<T>) -> Self {
        Self { cpu, sub_cpu, bus }
    }

    /// The composed RGBA framebuffer from the last rendered frame.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.bus.display_framebuffer()
    }

    /// The valid `(width, height)` of the composed framebuffer.
    pub fn display_dimensions(&self) -> (u32, u32) {
        self.bus.display_dimensions()
    }

    /// Interleaves the V30 and the floppy sub-CPU for up to `budget` main-clock
    /// cycles, returning the cycles actually advanced (which may slightly exceed
    /// `budget` when an instruction completes past the boundary).
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        if T::ENABLED {
            self.run_sub_for_main_cycles(0);
            if self.bus.tracer().yield_requested() {
                return 0;
            }
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let resynchronizing = current_cycle < self.bus.resync_until;

            // When the V30 is halted and no handshake is in flight, fast-forward
            // to the next scheduled event so an IRQ can wake it, instead of
            // stepping one tiny slice at a time. The sub CPU still runs for the
            // elapsed time below.
            let slice_cycles = if self.cpu.halted() && !resynchronizing {
                let next_event_cycle = self.bus.next_event_cycle().unwrap_or(target_cycle);
                next_event_cycle.clamp(current_cycle + 1, target_cycle) - current_cycle
            } else {
                let slice_cap = if resynchronizing {
                    SYNC_SLICE
                } else {
                    TIGHT_SLICE
                };
                (target_cycle - current_cycle).min(slice_cap).max(1)
            };
            let slice_end = current_cycle + slice_cycles;

            self.run_main_until(slice_end);
            let elapsed_cycles = self.bus.current_cycle() - current_cycle;
            if T::ENABLED && self.bus.tracer().yield_requested() {
                self.bus.sub_clock_credit =
                    self.bus.sub_clock_credit.saturating_add(elapsed_cycles);
                break;
            }

            // Run the sub CPU for the same elapsed wall-clock, converted to its
            // T-state domain by the clock ratio.
            self.run_sub_for_main_cycles(elapsed_cycles);
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    /// Advances the main V30 to at least `slice_end`, idling a halted core so its
    /// scheduled events still fire and an interrupt can wake it.
    fn run_main_until(&mut self, slice_end: u64) {
        let current_cycle = self.bus.current_cycle();
        if current_cycle >= slice_end {
            return;
        }
        let ran_cycles = self.cpu.run_for(slice_end - current_cycle, &mut self.bus);

        // A halted (or fully idle) CPU consumes nothing: advance to the slice end
        // as idle so the sub CPU still runs and interrupts can wake the core.
        if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
            self.bus.set_current_cycle(slice_end);
        }
    }

    /// Runs the sub CPU for `main_cycles` of elapsed main-clock time, converting to
    /// sub T-states and carrying the remainder for an exact long-run ratio.
    fn run_sub_for_main_cycles(&mut self, main_cycles: u64) {
        let shift = self.bus.sub_to_main_shift;
        let available_cycles = main_cycles + self.bus.sub_clock_credit;
        let sub_cycles = available_cycles >> shift;
        self.bus.sub_clock_credit = available_cycles - (sub_cycles << shift);
        if sub_cycles == 0 {
            return;
        }
        let mut view = SubBusView { bus: &mut self.bus };
        let ran_cycles = self.sub_cpu.run_for(sub_cycles, &mut view);
        if T::ENABLED && ran_cycles < sub_cycles {
            let remaining_cycles = (sub_cycles - ran_cycles)
                .checked_shl(shift)
                .unwrap_or(u64::MAX);
            self.bus.sub_clock_credit = self.bus.sub_clock_credit.saturating_add(remaining_cycles);
        }
    }

    /// Mounts a floppy image into a drive with the requested backing.
    pub fn insert_floppy(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        let parsed =
            device::floppy::load_floppy_image(std::path::Path::new(image.name), image.bytes)
                .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
        let description = format!("{} ({})", parsed.name, parsed.format_name());
        self.bus.insert_floppy_backed(drive, parsed, backing);
        Ok(description)
    }

    /// Returns the current in-memory bytes of the floppy in `drive`, if mounted.
    pub fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.floppy_image_bytes(drive)
    }

    /// Ejects the floppy from a drive.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    /// Flushes any dirty mounted floppies back to their source files.
    pub fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = Pc88VaRuntimeState {
            main_cpu: self.cpu.capture_state(),
            sub_cpu: self.sub_cpu.capture_state(),
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
                Ok(Pc88VaRuntimeState {
                    main_cpu: machine.cpu.capture_state(),
                    sub_cpu: machine.sub_cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.cpu.restore_state(state.main_cpu)?;
                machine.sub_cpu.restore_state(state.sub_cpu)?;
                machine.bus.restore_runtime_state(state.bus)
            },
        )
    }
}

impl Pc88VaMachine<NoTrace> {
    /// Builds a reset V30 for the PC-88VA reset vector.
    pub fn reset_cpu() -> cpu::V30 {
        reset_cpu()
    }
}

/// Builds a reset V30 for the PC-88VA reset vector.
pub fn reset_cpu() -> cpu::V30 {
    let mut cpu = cpu::V30::new();
    cpu.reset();
    cpu.set_ip(RESET_IP);
    cpu.set_cs(RESET_CS);
    cpu
}

impl<T: TraceSink> common::Machine for Pc88VaMachine<T> {
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

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.clock_config().main_clock_hz)
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc88VaMachine::run_for(self, budget)
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
        self.bus.push_key_scancode(code);
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
            HostKey::KpEnter => 0x79,
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

    fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.bus.push_mouse_delta(delta_x, delta_y);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        // The VA mouse has two buttons; the middle button is ignored.
        self.bus.set_mouse_buttons(left, right);
    }

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        // The VA exposes a single joystick port; ignore higher indices.
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
        Pc88VaMachine::insert_floppy(self, drive, image, backing)
    }

    fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        Pc88VaMachine::floppy_image_bytes(self, drive)
    }

    fn eject_floppy(&mut self, drive: usize) {
        Pc88VaMachine::eject_floppy(self, drive);
    }

    fn flush_floppies(&mut self) {
        Pc88VaMachine::flush_floppies(self);
    }

    fn insert_cdrom(&mut self, _path: &std::path::Path) -> Result<String, String> {
        Err("the PC-88VA2 has no CD-ROM drive".into())
    }

    fn eject_cdrom(&mut self) {}
}

#[cfg(test)]
mod tests {
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use super::{Pc88VaMachine, reset_cpu};
    use crate::{bus::Pc88VaBus, config::Pc88VaModel, rom::LoadedRoms};

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
        let model = Pc88VaModel::PC88VA2;
        let roms = LoadedRoms {
            rom00: vec![0; 0x8_0000],
            rom08: vec![0; 0x2_0000],
            rom1: vec![0; 0x2_0000],
            font: vec![0; 0x5_0000],
            dictionary: vec![0; 0x8_0000],
            subsys: vec![0; 0x2000],
        };
        let bus = Pc88VaBus::new_with_trace_sink(model, roms, 48_000, YieldOnScheduled::default());
        let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
        let mut machine = Pc88VaMachine::new(reset_cpu(), sub_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn main_trace_yield_preserves_sub_cpu_clock_debt() {
        let model = Pc88VaModel::PC88VA2;
        let roms = LoadedRoms {
            rom00: vec![0; 0x8_0000],
            rom08: vec![0; 0x2_0000],
            rom1: vec![0; 0x2_0000],
            font: vec![0; 0x5_0000],
            dictionary: vec![0; 0x8_0000],
            subsys: vec![0; 0x2000],
        };
        let bus = Pc88VaBus::new_with_trace_sink(model, roms, 48_000, YieldOnMainFetch::default());
        let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
        let mut machine = Pc88VaMachine::new(reset_cpu(), sub_cpu, bus);
        machine.bus.tracer_mut().arm();

        machine.run_for(100);

        let shift = machine.bus.sub_to_main_shift;
        let pending_tstates = machine.bus.sub_clock_credit >> shift;
        let sub_cycle_before_resume = machine.bus.sub_cycle;
        assert!(pending_tstates > 0);

        machine.bus.tracer_mut().resume();
        assert_eq!(machine.run_for(0), 0);

        assert!(machine.bus.sub_cycle >= sub_cycle_before_resume + pending_tstates);
        assert!(machine.bus.sub_clock_credit < 1 << shift);
    }
}
