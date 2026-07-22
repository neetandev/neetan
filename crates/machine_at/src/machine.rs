//! PC/AT machine: an i486 main CPU on the full 32-bit memory map.

use common::{Bus, Cpu, HostKey, KeyModifiers, Machine, NoTrace, StartupCapabilities, TraceSink};

use crate::{bus::AtBus, config::AtBootDevice};

/// CPU cycles executed per interleave slice while the CPU is running. Kept
/// tight so scheduled timer interrupts are serviced promptly.
const TIGHT_SLICE: u64 = 64;

save_state::runtime_state! {
/// Machine-root state for one PC/AT snapshot.
#[derive(Clone)]
struct AtRuntimeState {
    cpu: cpu::I386State,
    bus: crate::bus::AtBusState,
}}

/// An IBM PC/AT machine: the i486 main CPU and the AT bus.
pub struct AtMachine<T: TraceSink = NoTrace> {
    /// The main CPU, on the 32-bit physical address map.
    pub cpu: cpu::I386<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>,
    /// The system bus, owning memory and devices.
    pub bus: AtBus<T>,
}

/// Builds an untraced PC/AT machine around a configured bus.
pub fn build_untraced_machine(bus: AtBus<NoTrace>, boot_device: AtBootDevice) -> Box<dyn Machine> {
    let mut cpu = cpu::I386::<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    let mut machine = AtMachine::new(cpu, bus);
    machine.set_boot_device(boot_device);
    Box::new(machine)
}

/// Builds an automated PC/AT machine around a configured bus.
pub fn build_automated_machine(
    bus: AtBus<common::tracing::ApplicationTraceSink>,
    boot_device: AtBootDevice,
) -> Box<dyn common::AutomatedMachine> {
    let mut cpu = cpu::I386::<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    let mut machine = AtMachine::new(cpu, bus);
    machine.set_boot_device(boot_device);
    Box::new(machine)
}

impl common::AutomationDriver for AtMachine<common::tracing::ApplicationTraceSink> {
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
        AtMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        false
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl common::AutomatedMachine for AtMachine<common::tracing::ApplicationTraceSink> {
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "at",
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
        AT_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the IBM PC/AT bus.
const AT_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::AT_PIC],
    scheduled: common::trace_id::scheduled::AT,
    devices: &[common::TraceDeviceCatalog {
        device: common::trace_id::device::AT_FDC,
        actions: &[common::trace_action(common::trace_id::action::READ)],
    }],
    providers: &[],
};

impl common::MachineInspector for AtMachine<common::tracing::ApplicationTraceSink> {
    fn processors(&self) -> common::ProcessorList {
        let mut processors = common::ProcessorList::new();
        processors.push(common::inspect::x86_processor("cpu.main", true));
        processors
    }

    fn address_spaces(&self) -> common::AddressSpaceList {
        let mut spaces = common::AddressSpaceList::new();
        spaces.push(common::inspect::memory_space(
            "cpu.main.memory",
            32,
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
            "cpu.main" => common::inspect::x86_read(&self.cpu, register),
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
            _ => Err(common::InspectError::UnknownProcessor),
        }
    }

    fn protected_mode_state(
        &self,
        processor: &str,
    ) -> Result<common::ProtectedModeState, common::InspectError> {
        match processor {
            "cpu.main" => self
                .cpu
                .protected_mode_state()
                .ok_or(common::InspectError::Unsupported),
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
                        .poke_byte(common::inspect::offset_u32(address, index)?, *byte);
                }
                Ok(())
            }
            "cpu.main.io" => Err(common::InspectError::NotWritable),
            _ => Err(common::InspectError::UnknownSpace),
        }
    }
}

impl<T: TraceSink> AtMachine<T> {
    /// Builds a machine around a configured CPU and bus.
    pub fn new(
        cpu: cpu::I386<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>,
        bus: AtBus<T>,
    ) -> Self {
        Self { cpu, bus }
    }

    /// Selects the BIOS boot device order in the CMOS.
    pub fn set_boot_device(&mut self, device: AtBootDevice) {
        self.bus.set_boot_device(device);
    }

    /// Runs the CPU for up to `budget` cycles, returning the cycles advanced.
    ///
    /// The CPU advances the bus clock per instruction, so scheduled events fire
    /// mid-slice; a halted CPU fast-forwards to the next event so an interrupt
    /// can wake it. A CS4031/KBC-requested reset resets only the CPU, leaving
    /// the chipset, RAM and CMOS intact (the AMI warm-boot path relies on this).
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let slice_end = if self.cpu.halted() {
                let next_event_cycle = self.bus.next_event_cycle().unwrap_or(target_cycle);
                next_event_cycle.clamp(current_cycle + 1, target_cycle)
            } else {
                current_cycle.saturating_add(TIGHT_SLICE).min(target_cycle)
            };

            let ran_cycles = self.cpu.run_for(slice_end - current_cycle, &mut self.bus);
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }

            if self.bus.reset_pending() {
                if self.bus.take_cpu_reset() {
                    self.cpu.reset();
                }
                continue;
            }

            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    fn capture_machine_blob(
        &mut self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = AtRuntimeState {
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
                Ok(AtRuntimeState {
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

impl<T: TraceSink> Machine for AtMachine<T> {
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
        AtMachine::run_for(self, budget)
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
            HostKey::LeftBracket => 0x1A,
            HostKey::RightBracket => 0x1B,
            HostKey::Return => 0x1C,
            HostKey::LeftControl => 0x1D,
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
            HostKey::Grave => 0x29,
            HostKey::LeftShift => 0x2A,
            HostKey::Backslash => 0x2B,
            HostKey::Z => 0x2C,
            HostKey::X => 0x2D,
            HostKey::C => 0x2E,
            HostKey::V => 0x2F,
            HostKey::B => 0x30,
            HostKey::N => 0x31,
            HostKey::M => 0x32,
            HostKey::Comma => 0x33,
            HostKey::Period => 0x34,
            HostKey::Slash => 0x35,
            HostKey::RightShift => 0x36,
            HostKey::KpMultiply => 0x37,
            HostKey::LeftAlt => 0x38,
            HostKey::Space => 0x39,
            HostKey::CapsLock => 0x3A,
            HostKey::F1 => 0x3B,
            HostKey::F2 => 0x3C,
            HostKey::F3 => 0x3D,
            HostKey::F4 => 0x3E,
            HostKey::F5 => 0x3F,
            HostKey::F6 => 0x40,
            HostKey::F7 => 0x41,
            HostKey::F8 => 0x42,
            HostKey::F9 => 0x43,
            HostKey::F10 => 0x44,
            HostKey::NumLock => 0x45,
            HostKey::Kp7 => 0x47,
            HostKey::Kp8 => 0x48,
            HostKey::Kp9 => 0x49,
            HostKey::KpMinus => 0x4A,
            HostKey::Kp4 => 0x4B,
            HostKey::Kp5 => 0x4C,
            HostKey::Kp6 => 0x4D,
            HostKey::KpPlus => 0x4E,
            HostKey::Kp1 => 0x4F,
            HostKey::Kp2 => 0x50,
            HostKey::Kp3 => 0x51,
            HostKey::Kp0 => 0x52,
            HostKey::KpPeriod => 0x53,
            HostKey::NonUsBackslash => 0x56,
            HostKey::F11 => 0x57,
            HostKey::F12 => 0x58,
            HostKey::International1 => 0x73,
            HostKey::International2 => 0x70,
            HostKey::International3 => 0x7D,
            HostKey::International4 => 0x79,
            HostKey::International5 => 0x7B,
            HostKey::Up => crate::AT_KEY_CURSOR_UP,
            HostKey::Down => crate::AT_KEY_CURSOR_DOWN,
            HostKey::Left => crate::AT_KEY_CURSOR_LEFT,
            HostKey::Right => crate::AT_KEY_CURSOR_RIGHT,
            HostKey::Insert => crate::AT_KEY_INSERT,
            HostKey::Delete => crate::AT_KEY_DELETE,
            HostKey::Home => crate::AT_KEY_HOME,
            HostKey::End => crate::AT_KEY_END,
            HostKey::PageUp => crate::AT_KEY_PAGE_UP,
            HostKey::PageDown => crate::AT_KEY_PAGE_DOWN,
            HostKey::KpEnter => crate::AT_KEY_KEYPAD_ENTER,
            HostKey::KpDivide => crate::AT_KEY_KEYPAD_DIVIDE,
            HostKey::RightControl => crate::AT_KEY_RIGHT_CTRL,
            HostKey::RightAlt => crate::AT_KEY_RIGHT_ALT,
            _ => return None,
        })
    }

    fn push_keyboard_scancode(&mut self, code: u8) {
        self.bus.push_key_scancode(code);
    }

    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        self.bus.set_mouse_buttons(left, right);
    }

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        self.bus.set_joystick(index, state);
    }

    fn set_joystick_axes(&mut self, index: usize, axes: Option<(i16, i16)>) {
        self.bus.set_joystick_axes(index, axes);
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn font_rom_data(&self) -> &[u8] {
        &[]
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
        self.bus.insert_floppy_backed(drive, parsed, backing)?;
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

    fn startup_capabilities(&self) -> StartupCapabilities {
        StartupCapabilities {
            hard_disk: true,
            mt32: true,
            sc55: true,
            ..StartupCapabilities::default()
        }
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

    fn insert_hdd(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        let parsed = device::disk::load_hdd_image(std::path::Path::new(image.name), image.bytes)
            .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
        let description = format!(
            "HDD{}: {}C/{}H/{}S ({}) from {}",
            drive + 1,
            parsed.geometry.cylinders,
            parsed.geometry.heads,
            parsed.geometry.sectors_per_track,
            parsed.format_name(),
            image.name,
        );
        self.bus.insert_hdd_backed(drive, parsed, backing)?;
        Ok(description)
    }

    fn hdd_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.hdd_image_bytes(drive)
    }

    fn flush_hdds(&mut self) {
        self.bus.flush_hdds();
    }

    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        let (image, description) = device::cdrom::load_cd_image(path)?;
        self.bus.insert_cdrom(image)?;
        Ok(description)
    }

    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
    }

    fn cd_audio_status(&self) -> Option<common::CdAudioStatus> {
        self.bus.cd_audio_status()
    }
}
