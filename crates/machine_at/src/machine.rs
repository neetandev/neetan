//! PC/AT machine: an i486 main CPU on the full 32-bit memory map.

use common::{Bus, Cpu, Machine, NoTrace, StartupCapabilities, TraceSink};

use crate::{bus::AtBus, config::AtBootDevice};

/// CPU cycles executed per interleave slice while the CPU is running. Kept
/// tight so scheduled timer interrupts are serviced promptly.
const TIGHT_SLICE: u64 = 64;

/// An IBM PC/AT machine: the i486 main CPU and the AT bus.
pub struct AtMachine<T: TraceSink = NoTrace> {
    /// The main CPU, on the 32-bit physical address map.
    pub cpu: cpu::I386<{ cpu::CPU_MODEL_486_DX }, { cpu::ADDRESS_WIDTH_32 }>,
    /// The system bus, owning memory and devices.
    pub bus: AtBus<T>,
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
    fn run_for_impl(&mut self, budget: u64) -> u64 {
        let start = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target = start + budget;
        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let slice_end = if self.cpu.halted() {
                let next = self.bus.next_event_cycle().unwrap_or(target);
                next.clamp(current + 1, target)
            } else {
                (current + TIGHT_SLICE).min(target)
            };

            let ran = self.cpu.run_for(slice_end - current, &mut self.bus);
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }

            if self.bus.reset_pending() {
                if self.bus.take_cpu_reset() {
                    self.cpu.reset();
                }
                continue;
            }

            if ran == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }
        self.bus.current_cycle() - start
    }
}

impl<T: TraceSink> Machine for AtMachine<T> {
    fn set_host_date_time_provider(&mut self, provider: common::HostDateTimeProvider) {
        self.bus.set_host_date_time_provider(provider);
    }

    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.cpu_clock_hz())
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        self.run_for_impl(budget)
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

    fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.bus.push_mouse_delta(dx, dy);
    }

    fn set_mouse_buttons(&mut self, left: bool, right: bool, _middle: bool) {
        self.bus.set_mouse_buttons(left, right);
    }

    fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        self.bus.set_joystick(index, state);
    }

    fn set_joystick_axes(&mut self, index: usize, x: i16, y: i16) {
        self.bus.set_joystick_axes(index, x, y);
    }

    fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        self.bus.generate_audio_samples(volume, output)
    }

    fn font_rom_data(&self) -> &[u8] {
        &[]
    }

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()))?;
        Ok(description)
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

    fn insert_hdd(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let image = device::disk::load_hdd_image(path, &data)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        let description = format!(
            "HDD{}: {}C/{}H/{}S ({}) from {}",
            drive + 1,
            image.geometry.cylinders,
            image.geometry.heads,
            image.geometry.sectors_per_track,
            image.format_name(),
            path.display(),
        );
        self.bus
            .insert_hdd(drive, image, Some(path.to_path_buf()))?;
        Ok(description)
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
