//! FM Towns machine: an i386/i486 main CPU on the full 32-bit memory map.
//!
//! A single monotonic `current_cycle` (CPU-clock units) drives the scheduler.
//! The machine is generic over the CPU model so the same bus serves the 486-based
//! MX and the 386-based CX; the physical address width is fixed at 32 bits for
//! both.

use common::{Bus, Cpu, HostKey, JoystickState, KeyModifiers, Machine, NoTrace, TraceSink};

use crate::{
    bus::TownsBus,
    config::{TownsBootDevice, TownsModel, TownsPadType},
};

/// CMOS boot-device type / boot-device byte pairs (I/O 0x3182 / 0x3C28).
const BOOT_CMOS_CD: (u8, u8) = (8, 0x80);
const BOOT_CMOS_FLOPPY: (u8, u8) = (2, 0x20);
const BOOT_CMOS_HDD: (u8, u8) = (1, 0x10);

/// CPU cycles executed per interleave slice while the CPU is running. Kept tight
/// so scheduled timer interrupts are serviced promptly.
const TIGHT_SLICE: u64 = 64;

save_state::runtime_state! {
/// Machine-root state for one FM Towns snapshot.
#[derive(Clone)]
struct TownsRuntimeState {
    cpu: cpu::I386State,
    bus: crate::bus::TownsBusState,
    boot_device: u8,
}}

/// An FM Towns machine: the i386/i486 main CPU and the Towns bus.
pub struct TownsMachine<const CPU_MODEL: u8, T: TraceSink = NoTrace> {
    /// The main CPU, on the 32-bit physical address map.
    pub cpu: cpu::I386<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>,
    /// The system bus, owning memory and devices.
    pub bus: TownsBus<T>,
    /// Requested boot device; resolved into the CMOS boot-device byte.
    boot_device: TownsBootDevice,
}

/// Builds an untraced FM Towns machine and selects the CPU for `model`.
pub fn build_untraced_machine(
    model: TownsModel,
    bus: TownsBus<NoTrace>,
    boot_device: TownsBootDevice,
    pad_type: TownsPadType,
    cdrom_compatibility_timing: bool,
) -> Box<dyn Machine> {
    match model {
        TownsModel::FmTowns => build_untraced_machine_for_cpu::<{ cpu::CPU_MODEL_386_SX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
        TownsModel::FmTownsIICx => build_untraced_machine_for_cpu::<{ cpu::CPU_MODEL_386_DX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
        TownsModel::FmTownsIIMx => build_untraced_machine_for_cpu::<{ cpu::CPU_MODEL_486_DX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
    }
}

/// Builds one concrete untraced FM Towns CPU variant.
fn build_untraced_machine_for_cpu<const CPU_MODEL: u8>(
    bus: TownsBus<NoTrace>,
    boot_device: TownsBootDevice,
    pad_type: TownsPadType,
    cdrom_compatibility_timing: bool,
) -> Box<dyn Machine> {
    let mut cpu = cpu::I386::<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    let mut machine = TownsMachine::new(cpu, bus);
    machine.set_boot_device(boot_device);
    machine.set_pad_type(pad_type);
    machine.set_cdrom_compatibility_timing(cdrom_compatibility_timing);
    Box::new(machine)
}

/// Builds an automated FM Towns machine and selects the CPU for `model`.
pub fn build_automated_machine(
    model: TownsModel,
    bus: TownsBus<common::tracing::ApplicationTraceSink>,
    boot_device: TownsBootDevice,
    pad_type: TownsPadType,
    cdrom_compatibility_timing: bool,
) -> Box<dyn common::AutomatedMachine> {
    match model {
        TownsModel::FmTowns => build_automated_machine_for_cpu::<{ cpu::CPU_MODEL_386_SX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
        TownsModel::FmTownsIICx => build_automated_machine_for_cpu::<{ cpu::CPU_MODEL_386_DX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
        TownsModel::FmTownsIIMx => build_automated_machine_for_cpu::<{ cpu::CPU_MODEL_486_DX }>(
            bus,
            boot_device,
            pad_type,
            cdrom_compatibility_timing,
        ),
    }
}

/// Builds one concrete automated FM Towns CPU variant.
fn build_automated_machine_for_cpu<const CPU_MODEL: u8>(
    bus: TownsBus<common::tracing::ApplicationTraceSink>,
    boot_device: TownsBootDevice,
    pad_type: TownsPadType,
    cdrom_compatibility_timing: bool,
) -> Box<dyn common::AutomatedMachine> {
    let mut cpu = cpu::I386::<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>::new();
    cpu.reset();
    let mut machine = TownsMachine::new(cpu, bus);
    machine.set_boot_device(boot_device);
    machine.set_pad_type(pad_type);
    machine.set_cdrom_compatibility_timing(cdrom_compatibility_timing);
    Box::new(machine)
}

impl<const CPU_MODEL: u8> common::AutomationDriver
    for TownsMachine<CPU_MODEL, common::tracing::ApplicationTraceSink>
{
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
        TownsMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.bus.power_off_requested()
    }

    fn drain_audio(&mut self, elapsed_ticks: u64) {
        self.bus.drain_automation_audio(elapsed_ticks);
    }
}

impl<const CPU_MODEL: u8> common::AutomatedMachine
    for TownsMachine<CPU_MODEL, common::tracing::ApplicationTraceSink>
{
    fn automation_descriptor(&self) -> common::AutomationDescriptor {
        let (numerator, denominator) = self.bus.automation_timebase();
        common::AutomationDescriptor {
            target: "towns",
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
        TOWNS_TRACE_CATALOG
    }
}

/// Stable trace identifiers emitted by the FM Towns bus.
const TOWNS_TRACE_CATALOG: common::TraceCatalog = common::TraceCatalog {
    controllers: &[common::trace_id::controller::TOWNS_PIC],
    scheduled: common::trace_id::scheduled::TOWNS,
    devices: &[
        common::TraceDeviceCatalog {
            device: common::trace_id::device::TOWNS_CDROM,
            actions: &[
                common::trace_action(common::trace_id::action::INTERRUPT),
                common::trace_action(common::trace_id::action::STATUS),
                common::trace_action(common::trace_id::action::COMMAND),
            ],
        },
        common::TraceDeviceCatalog {
            device: common::trace_id::device::TOWNS_FDC,
            actions: &[common::trace_action(common::trace_id::action::READ)],
        },
    ],
    providers: &[],
};

impl<const CPU_MODEL: u8> common::MachineInspector
    for TownsMachine<CPU_MODEL, common::tracing::ApplicationTraceSink>
{
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

impl<const CPU_MODEL: u8, T: TraceSink> TownsMachine<CPU_MODEL, T> {
    /// Builds a machine around a configured CPU and bus.
    pub fn new(cpu: cpu::I386<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>, bus: TownsBus<T>) -> Self {
        Self {
            cpu,
            bus,
            boot_device: TownsBootDevice::default(),
        }
    }
}

impl<const CPU_MODEL: u8, T: TraceSink> TownsMachine<CPU_MODEL, T> {
    /// Selects the boot device and writes the resolved CMOS boot-device byte.
    pub fn set_boot_device(&mut self, boot_device: TownsBootDevice) {
        self.boot_device = boot_device;
        self.refresh_boot_device();
    }

    /// Selects the pad type plugged into game port 0.
    pub fn set_pad_type(&mut self, pad_type: TownsPadType) {
        self.bus.set_pad_type(pad_type);
    }

    /// Enables or disables the CD-ROM drive's slow compatibility timing.
    pub fn set_cdrom_compatibility_timing(&mut self, enabled: bool) {
        self.bus.set_cdrom_compatibility_timing(enabled);
    }

    /// Installs a Roland MT-32 sound module driven by RS-MIDI (RS-232C) output.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.bus.install_mt32(rom_directory)
    }

    /// Installs a Roland SC-55 sound module driven by RS-MIDI (RS-232C) output.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.bus.install_sc55(rom_directory)
    }

    /// Attaches a hard disk image at the given SCSI drive index (0-based).
    pub fn insert_hdd(
        &mut self,
        drive: usize,
        image: device::disk::HddImage,
        path: Option<std::path::PathBuf>,
    ) {
        self.insert_hdd_backed(drive, image, path.into());
    }

    /// Attaches a hard disk image with the requested backing.
    pub fn insert_hdd_backed(
        &mut self,
        drive: usize,
        image: device::disk::HddImage,
        backing: common::MediaBacking,
    ) {
        self.bus.insert_hdd_backed(drive, image, backing);
        self.refresh_boot_device();
    }

    /// Resolves the requested boot device (CD-when-present for `Auto`) and writes
    /// the CMOS boot-device bytes the SYSROM IPL reads.
    fn refresh_boot_device(&mut self) {
        let (device_type, boot_device) = match self.boot_device {
            TownsBootDevice::Cd => BOOT_CMOS_CD,
            TownsBootDevice::Floppy => BOOT_CMOS_FLOPPY,
            TownsBootDevice::Hdd => BOOT_CMOS_HDD,
            TownsBootDevice::Auto => {
                // Match the real IPL's search order: CD-ROM, then SCSI hard
                // disk, then floppy.
                if self.bus.has_cdrom() {
                    BOOT_CMOS_CD
                } else if self.bus.has_hdd() {
                    BOOT_CMOS_HDD
                } else {
                    BOOT_CMOS_FLOPPY
                }
            }
        };
        self.bus.set_boot_device_cmos(device_type, boot_device);
    }

    /// Runs the CPU for up to `budget` cycles, returning the cycles advanced.
    /// The CPU advances the bus clock per instruction, so scheduled events fire
    /// mid-slice; a halted CPU fast-forwards to the next event so an interrupt
    /// can wake it.
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

            // A soft reset or power-off requested through I/O 0x0020/0x0022
            // breaks the CPU out of its slice: stop for a shutdown, or reset the
            // CPU and restore the power-on banking for a soft reset.
            if self.bus.reset_pending() {
                if self.bus.power_off_requested() {
                    break;
                }
                if self.bus.take_soft_reset() {
                    self.cpu.reset();
                    self.bus.memory.reset_banking();
                }
                continue;
            }

            // A halted or fully idle CPU consumes nothing: advance time to the
            // slice end so scheduled events fire and can wake the core.
            if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    fn capture_machine_blob(
        &mut self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = TownsRuntimeState {
            cpu: self.cpu.capture_state(),
            bus: self.bus.capture_runtime_state()?,
            boot_device: match self.boot_device {
                TownsBootDevice::Auto => 0,
                TownsBootDevice::Floppy => 1,
                TownsBootDevice::Hdd => 2,
                TownsBootDevice::Cd => 3,
            },
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
                Ok(TownsRuntimeState {
                    cpu: machine.cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                    boot_device: match machine.boot_device {
                        TownsBootDevice::Auto => 0,
                        TownsBootDevice::Floppy => 1,
                        TownsBootDevice::Hdd => 2,
                        TownsBootDevice::Cd => 3,
                    },
                })
            },
            |machine, state| {
                let boot_device = match state.boot_device {
                    0 => TownsBootDevice::Auto,
                    1 => TownsBootDevice::Floppy,
                    2 => TownsBootDevice::Hdd,
                    3 => TownsBootDevice::Cd,
                    _ => {
                        return Err(save_state::StateValidationError::new(
                            "FM Towns boot device is invalid",
                        )
                        .into());
                    }
                };
                machine.cpu.restore_state(state.cpu)?;
                machine.bus.restore_runtime_state(state.bus)?;
                machine.boot_device = boot_device;
                Ok(())
            },
        )
    }
}

impl<const CPU_MODEL: u8, T: TraceSink> Machine for TownsMachine<CPU_MODEL, T> {
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
            hard_disk: true,
            mt32: true,
            sc55: true,
            ..Default::default()
        }
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.clocks.cpu_clock_hz)
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        TownsMachine::run_for(self, budget)
    }

    fn shutdown_requested(&self) -> bool {
        self.bus.power_off_requested()
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
            HostKey::Kp1 => 0x42,
            HostKey::Kp2 => 0x43,
            HostKey::Kp3 => 0x44,
            HostKey::KpEnter => 0x45,
            HostKey::Kp0 => 0x46,
            HostKey::KpPeriod => 0x47,
            HostKey::Insert => 0x48,
            HostKey::Delete => 0x4B,
            HostKey::Up => 0x4D,
            HostKey::Home => 0x4E,
            HostKey::Left => 0x4F,
            HostKey::Down => 0x50,
            HostKey::Right => 0x51,
            HostKey::LeftControl => 0x52,
            HostKey::RightControl => 0x52,
            HostKey::LeftShift => 0x53,
            HostKey::RightShift => 0x53,
            HostKey::CapsLock => 0x55,
            HostKey::F12 => 0x5B,
            HostKey::LeftAlt => 0x5C,
            HostKey::RightAlt => 0x5C,
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
            HostKey::F11 => 0x69,
            HostKey::End => 0x72,
            HostKey::PageDown => 0x73,
            HostKey::Pause => 0x7C,
            HostKey::PrintScreen => 0x7D,
            _ => return None,
        })
    }

    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        self.bus.set_mouse_buttons(left, right);
    }

    fn set_joystick(&mut self, index: usize, state: JoystickState) {
        // The FM Towns has one game pad on port 0.
        if index == 0 {
            self.bus.set_pad(state);
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
        let description = insert_floppy_impl(&mut self.bus, drive, image, backing)?;
        self.refresh_boot_device();
        Ok(description)
    }

    fn floppy_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.floppy_image_bytes(drive)
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
        self.refresh_boot_device();
    }

    fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }

    fn flush_hdds(&mut self) {
        self.bus.flush_hdds();
    }

    fn hdd_image_bytes(&self, drive: usize) -> Option<Vec<u8>> {
        self.bus.hdd_image_bytes(drive)
    }

    fn insert_hdd(
        &mut self,
        drive: usize,
        image: common::MediaImage<'_>,
        backing: common::MediaBacking,
    ) -> Result<String, String> {
        let parsed = device::disk::load_hdd_image(std::path::Path::new(image.name), image.bytes)
            .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
        if parsed.format != device::disk::HddFormat::Raw {
            return Err(format!(
                "FM Towns hard disks must be raw images (.h0-.h4); {} is {}",
                image.name,
                parsed.format_name(),
            ));
        }
        let description = format!(
            "FM Towns SCSI ID {drive}: {} sectors ({}) from {}",
            parsed.geometry.total_sectors(),
            parsed.format_name(),
            image.name,
        );
        TownsMachine::insert_hdd_backed(self, drive, parsed, backing);
        Ok(description)
    }

    #[cfg(feature = "mt32")]
    fn install_mt32(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        TownsMachine::install_mt32(self, rom_directory).map_err(|error| error.to_string())
    }

    #[cfg(feature = "sc55")]
    fn install_sc55(&mut self, rom_directory: &std::path::Path) -> Result<(), String> {
        TownsMachine::install_sc55(self, rom_directory).map_err(|error| error.to_string())
    }

    fn insert_cdrom(&mut self, path: &std::path::Path) -> Result<String, String> {
        let description = insert_cdrom_impl(&mut self.bus, path)?;
        self.refresh_boot_device();
        Ok(description)
    }

    fn eject_cdrom(&mut self) {
        self.bus.eject_cdrom();
        self.refresh_boot_device();
    }
}

/// Loads a floppy image (auto-detected by extension) and inserts it into the
/// given drive, returning a short description.
fn insert_floppy_impl<T: TraceSink>(
    bus: &mut TownsBus<T>,
    drive: usize,
    image: common::MediaImage<'_>,
    backing: common::MediaBacking,
) -> Result<String, String> {
    let parsed = device::floppy::load_floppy_image(std::path::Path::new(image.name), image.bytes)
        .map_err(|error| format!("Failed to parse {}: {error}", image.name))?;
    let description = format!("{} ({})", parsed.name, parsed.format_name());
    bus.insert_floppy_backed(drive, parsed, backing);
    Ok(description)
}

/// Loads a CD-ROM disc image (`.cue` or `.ccd`) and inserts it into the bus,
/// returning a short description.
fn insert_cdrom_impl<T: TraceSink>(
    bus: &mut TownsBus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let (image, description) = device::cdrom::load_cd_image(path)?;
    bus.insert_cdrom(image);
    Ok(description)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_roms() -> crate::LoadedRoms {
        crate::LoadedRoms {
            dos: Vec::new(),
            font: Vec::new(),
            system: Vec::new(),
            f20: Vec::new(),
            dictionary: Vec::new(),
            serial: Vec::new(),
        }
    }

    #[test]
    fn untraced_factory_builds_every_cpu_model() {
        for model in [
            TownsModel::FmTowns,
            TownsModel::FmTownsIICx,
            TownsModel::FmTownsIIMx,
        ] {
            let bus = TownsBus::new(model, common::CpuMode::High, empty_roms(), 48_000);
            let machine = build_untraced_machine(
                model,
                bus,
                TownsBootDevice::Auto,
                TownsPadType::SixButton,
                false,
            );
            assert_eq!(
                machine.cpu_clock_hz() as u32,
                model.cpu_clock_hz(common::CpuMode::High)
            );
        }
    }
}
