//! Top-level PC-8801 machine.
//!
//! Wires the main Z80 and the disk sub-CPU (PC80S31K) to the shared bus and
//! interleaves them for a cycle budget. A single monotonic `current_cycle` (in
//! main-clock units) drives the scheduler; the sub CPU runs proportional
//! T-states converted by the clock ratio. The slice is kept tight, and tighter
//! still while a PPI mailbox handshake is in flight, so the two cores make
//! progress together.

use common::{CpuZ80, HostKey, KeyModifiers, NoTrace, TraceSink};

use crate::bus::{MainBusView, Pc8801Bus, SYNC_SLICE, SubBusView, TIGHT_SLICE};

save_state::runtime_state! {
/// Machine-root state for one PC-8801 snapshot.
#[derive(Clone)]
struct Pc8801RuntimeState {
    main_cpu: cpu::Z80State,
    sub_cpu: cpu::Z80State,
    bus: crate::bus::Pc8801BusState,
}}

/// PC-8801 machine: the main Z80 and the disk sub-CPU sharing one bus.
pub struct Pc8801Machine<T: TraceSink = NoTrace> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// Disk sub-CPU (PC80S31K).
    pub sub_cpu: cpu::Z80,
    /// System bus.
    pub bus: Pc8801Bus<T>,
}

/// Builds an untraced PC-8801 machine around a configured bus.
pub fn build_untraced_machine(bus: Pc8801Bus<NoTrace>) -> Box<dyn common::Machine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
    Box::new(Pc8801Machine::new(main_cpu, sub_cpu, bus))
}

/// Builds an automated PC-8801 machine around a configured bus.
pub fn build_automated_machine(
    bus: Pc8801Bus<common::tracing::ApplicationTraceSink>,
) -> Box<dyn common::AutomatedMachine> {
    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
    Box::new(Pc8801Machine::new(main_cpu, sub_cpu, bus))
}

impl common::AutomationDriver for Pc8801Machine<common::tracing::ApplicationTraceSink> {
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
        Pc8801Machine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for Pc8801Machine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "pc88",
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
        PC88_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the PC-88 bus.
const PC88_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[
        common::trace_id::controller::PC88_I8214,
        common::trace_id::controller::PC88_SUB_FDC,
    ],
    scheduled: common::trace_id::scheduled::PC88,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::PC88_FDC,
        actions: &[common::trace_id::action::READ],
    }],
    providers: &[],
};

impl common::MachineInspector for Pc8801Machine<common::tracing::ApplicationTraceSink> {
    fn processors(&self) -> common::ProcessorList {
        let mut processors = common::ProcessorList::new();
        processors.push(common::inspect::z80_processor("cpu.main"));
        processors.push(common::inspect::z80_processor("cpu.sub"));
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
            "cpu.main" => common::inspect::z80_read(&self.main_cpu, register),
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
            "cpu.main" => common::inspect::z80_write(&mut self.main_cpu, register, value),
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
            "cpu.main.io" | "cpu.sub.io" => Err(common::InspectError::NotWritable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

impl<T: TraceSink> Pc8801Machine<T> {
    /// Creates a new machine from the given CPUs and bus.
    pub fn new(main_cpu: cpu::Z80, sub_cpu: cpu::Z80, bus: Pc8801Bus<T>) -> Self {
        Self {
            main_cpu,
            sub_cpu,
            bus,
        }
    }

    /// Interleaves the two CPUs for up to `budget` main-clock cycles, returning
    /// the cycles actually advanced (which may slightly exceed `budget` when an
    /// instruction completes past the boundary).
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
            let slice_cap = if current_cycle < self.bus.resync_until {
                SYNC_SLICE
            } else {
                TIGHT_SLICE
            };
            let slice_cycles = (target_cycle - current_cycle).min(slice_cap).max(1);
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

    /// Advances the main CPU to at least `slice_end`, honoring the V1S/N text-DMA
    /// BUSREQ lockout and guaranteeing forward progress when the CPU is halted.
    fn run_main_until(&mut self, slice_end: u64) {
        let current_cycle = self.bus.current_cycle();
        if current_cycle >= slice_end {
            return;
        }

        // The text DMA holds the bus (V1S/N display lockout): the main CPU cannot
        // execute, so advance time so its scheduled events still fire.
        if self.bus.busreq_active() {
            let stall_end = self
                .bus
                .busreq_until()
                .min(slice_end)
                .max(current_cycle + 1);
            self.bus.set_current_cycle(stall_end);
            return;
        }

        let ran_cycles = {
            let mut view = MainBusView { bus: &mut self.bus };
            self.main_cpu.run_for(slice_end - current_cycle, &mut view)
        };

        // A halted (or fully waited) CPU consumes nothing: advance to the slice
        // end as idle so the sub CPU still runs and interrupts can wake the core.
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

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = Pc8801RuntimeState {
            main_cpu: self.main_cpu.capture_state(),
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
                Ok(Pc8801RuntimeState {
                    main_cpu: machine.main_cpu.capture_state(),
                    sub_cpu: machine.sub_cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.main_cpu.restore_state(state.main_cpu)?;
                machine.sub_cpu.restore_state(state.sub_cpu)?;
                machine.bus.restore_runtime_state(state.bus)
            },
        )
    }
}

impl<T: TraceSink> common::Machine for Pc8801Machine<T> {
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
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        Pc8801Machine::run_for(self, budget)
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
        // The code encodes a 16x8 matrix cell as `row << 3 | column`, with bit 7
        // set for a release (matching the host-side key map).
        let pressed = code & 0x80 == 0;
        let cell = usize::from(code & 0x7F);
        self.bus.set_key(cell >> 3, cell & 0x07, pressed);
    }

    fn translate_host_key(&self, key: HostKey, _modifiers: KeyModifiers) -> Option<u8> {
        Some(match key {
            HostKey::Kp0 => 0x00,
            HostKey::Kp1 => 0x01,
            HostKey::Kp2 => 0x02,
            HostKey::Kp3 => 0x03,
            HostKey::Kp4 => 0x04,
            HostKey::Kp5 => 0x05,
            HostKey::Kp6 => 0x06,
            HostKey::Kp7 => 0x07,
            HostKey::Kp8 => 0x08,
            HostKey::Kp9 => 0x09,
            HostKey::KpMultiply => 0x0A,
            HostKey::KpPlus => 0x0B,
            HostKey::KpComma => 0x0D,
            HostKey::KpPeriod => 0x0E,
            HostKey::Return => 0x0F,
            HostKey::KpEnter => 0x0F,
            HostKey::LeftBracket => 0x10,
            HostKey::A => 0x11,
            HostKey::B => 0x12,
            HostKey::C => 0x13,
            HostKey::D => 0x14,
            HostKey::E => 0x15,
            HostKey::F => 0x16,
            HostKey::G => 0x17,
            HostKey::H => 0x18,
            HostKey::I => 0x19,
            HostKey::J => 0x1A,
            HostKey::K => 0x1B,
            HostKey::L => 0x1C,
            HostKey::M => 0x1D,
            HostKey::N => 0x1E,
            HostKey::O => 0x1F,
            HostKey::P => 0x20,
            HostKey::Q => 0x21,
            HostKey::R => 0x22,
            HostKey::S => 0x23,
            HostKey::T => 0x24,
            HostKey::U => 0x25,
            HostKey::V => 0x26,
            HostKey::W => 0x27,
            HostKey::X => 0x28,
            HostKey::Y => 0x29,
            HostKey::Z => 0x2A,
            HostKey::RightBracket => 0x2B,
            HostKey::NonUsBackslash => 0x2C,
            HostKey::Backslash => 0x2D,
            HostKey::Equals => 0x2E,
            HostKey::Minus => 0x2F,
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
            HostKey::Apostrophe => 0x3A,
            HostKey::Semicolon => 0x3B,
            HostKey::Comma => 0x3C,
            HostKey::Period => 0x3D,
            HostKey::Slash => 0x3E,
            HostKey::Home => 0x40,
            HostKey::Up => 0x41,
            HostKey::Right => 0x42,
            HostKey::Backspace => 0x43,
            HostKey::Delete => 0x43,
            HostKey::LeftAlt => 0x44,
            HostKey::RightAlt => 0x45,
            HostKey::LeftShift => 0x46,
            HostKey::RightShift => 0x46,
            HostKey::LeftControl => 0x47,
            HostKey::Pause => 0x48,
            HostKey::F1 => 0x49,
            HostKey::F2 => 0x4A,
            HostKey::F3 => 0x4B,
            HostKey::F4 => 0x4C,
            HostKey::F5 => 0x4D,
            HostKey::Space => 0x4E,
            HostKey::Escape => 0x4F,
            HostKey::Tab => 0x50,
            HostKey::Down => 0x51,
            HostKey::Left => 0x52,
            HostKey::End => 0x53,
            HostKey::PrintScreen => 0x54,
            HostKey::KpMinus => 0x55,
            HostKey::KpDivide => 0x56,
            HostKey::CapsLock => 0x57,
            HostKey::PageUp => 0x58,
            HostKey::PageDown => 0x59,
            HostKey::F6 => 0x68,
            HostKey::F7 => 0x69,
            HostKey::F8 => 0x6A,
            HostKey::F9 => 0x6B,
            HostKey::F10 => 0x6C,
            HostKey::F11 => 0x6D,
            HostKey::F12 => 0x6E,
            _ => return None,
        })
    }

    fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.bus.set_mouse_delta(delta_x, delta_y);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        // The PC-88 mouse has two buttons; the middle button is ignored.
        self.bus.set_mouse_buttons(left, right);
    }

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        // The bare MA exposes a single joystick port; ignore higher indices.
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
                .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
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

    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        insert_cdrom_impl(&mut self.bus, path)
    }

    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
    }
}

fn insert_cdrom_impl<T: TraceSink>(
    bus: &mut Pc8801Bus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let (image, description) = device::cdrom::load_cd_image(path)?;
    bus.insert_cdrom(image);
    Ok(description)
}

#[cfg(test)]
mod tests {
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use crate::{
        bus::Pc8801Bus,
        config::{ClockSelect, Pc8801Model},
        machine::Pc8801Machine,
    };

    fn machine() -> Pc8801Machine {
        let bus = Pc8801Bus::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000);
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
        Pc8801Machine::new(main_cpu, sub_cpu, bus)
    }

    #[test]
    fn busreq_window_idles_the_cpu() {
        let mut machine = machine();
        // Cancel the power-on display event so it does not perturb the window.
        machine
            .bus
            .scheduler
            .cancel(crate::scheduler::Event88::CrtcDisplayStart);
        machine
            .bus
            .scheduler
            .cancel(crate::scheduler::Event88::ClockTimer);
        machine.bus.next_event_cycle = u64::MAX;

        let start_pc = machine.main_cpu.state.pc;
        machine.bus.busreq_until = machine.bus.current_cycle() + 1_000;

        let ran = machine.run_for(500);

        assert_eq!(ran, 500, "the budget is consumed as idle cycles");
        assert_eq!(machine.bus.current_cycle(), 500, "time advanced");
        assert_eq!(
            machine.main_cpu.state.pc, start_pc,
            "the CPU does not execute while the bus is held"
        );
    }

    #[test]
    fn keyboard_scancode_decodes_into_the_matrix() {
        use common::Machine;
        let mut machine = machine();

        // Matrix row 3, column 2 encodes to (3 << 3) | 2 = 0x1A.
        machine.push_keyboard_scancode(0x1A);
        assert_eq!(machine.bus.io_read(0x03).0, 0b1111_1011);

        // Bit 7 set marks a release.
        machine.push_keyboard_scancode(0x1A | 0x80);
        assert_eq!(machine.bus.io_read(0x03).0, 0xFF);
    }

    #[test]
    fn cpu_runs_when_no_busreq() {
        let mut machine = machine();
        machine
            .bus
            .scheduler
            .cancel(crate::scheduler::Event88::CrtcDisplayStart);
        machine
            .bus
            .scheduler
            .cancel(crate::scheduler::Event88::ClockTimer);
        machine.bus.next_event_cycle = u64::MAX;

        // RAM reads as 0x00 (NOP) at reset, so the CPU advances PC.
        let start_pc = machine.main_cpu.state.pc;
        machine.run_for(100);

        assert_ne!(
            machine.main_cpu.state.pc, start_pc,
            "the CPU executes when the bus is free"
        );
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
        let model = Pc8801Model::PC8801MC;
        let bus = Pc8801Bus::new_with_trace_sink(
            model,
            ClockSelect::FourMhz,
            48_000,
            YieldOnScheduled::default(),
        );
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
        let mut machine = Pc8801Machine::new(main_cpu, sub_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn main_trace_yield_preserves_sub_cpu_clock_debt() {
        let model = Pc8801Model::PC8801MC;
        let bus = Pc8801Bus::new_with_trace_sink(
            model,
            ClockSelect::FourMhz,
            48_000,
            YieldOnMainFetch::default(),
        );
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
        let mut machine = Pc8801Machine::new(main_cpu, sub_cpu, bus);
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
