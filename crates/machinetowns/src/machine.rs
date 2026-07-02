//! FM Towns machine: an i386/i486 main CPU on the full 32-bit memory map.
//!
//! A single monotonic `current_cycle` (CPU-clock units) drives the scheduler.
//! The machine is generic over the CPU model so the same bus serves the 486-based
//! MX and the 386-based CX; the physical address width is fixed at 32 bits for
//! both.

use common::{Bus, Cpu, JoystickState, Machine, NoTracing, Tracing};

use crate::{
    bus::TownsBus,
    config::{ClockConfig, TownsBootDevice, TownsModel, TownsPadType},
    memory::TownsMemory,
    rom::LoadedRoms,
};

/// CMOS boot-device type / boot-device byte pairs (I/O 0x3182 / 0x3C28).
const BOOT_CMOS_CD: (u8, u8) = (8, 0x80);
const BOOT_CMOS_FLOPPY: (u8, u8) = (2, 0x20);
const BOOT_CMOS_HDD: (u8, u8) = (1, 0x10);

/// Default audio sample rate.
const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// CPU cycles executed per interleave slice while the CPU is running. Kept tight
/// so scheduled timer interrupts are serviced promptly.
const TIGHT_SLICE: u64 = 64;

/// An FM Towns machine: the i386/i486 main CPU and the Towns bus.
pub struct TownsMachine<const CPU_MODEL: u8, T: Tracing = NoTracing> {
    /// The main CPU, on the 32-bit physical address map.
    pub cpu: cpu::I386<CPU_MODEL, { cpu::ADDRESS_WIDTH_32 }>,
    /// The system bus, owning memory and devices.
    pub bus: TownsBus<T>,
    /// Requested boot device; resolved into the CMOS boot-device byte.
    boot_device: TownsBootDevice,
}

impl<const CPU_MODEL: u8, T: Tracing + Default> TownsMachine<CPU_MODEL, T> {
    /// Builds a machine for `model` from a loaded ROM set and points the CPU at
    /// its reset vector (0xFFFFFFF0, inside the SYSROM).
    pub fn new(model: TownsModel, cpu_mode: common::CpuMode, roms: LoadedRoms) -> Self {
        let clocks = ClockConfig {
            cpu_clock_hz: model.cpu_clock_hz(cpu_mode),
            sample_rate: DEFAULT_SAMPLE_RATE,
        };
        let memory = TownsMemory::new(model, roms);
        let bus = TownsBus::new(memory, clocks, model);

        let mut cpu = cpu::I386::new();
        cpu.reset();

        Self {
            cpu,
            bus,
            boot_device: TownsBootDevice::default(),
        }
    }
}

impl<const CPU_MODEL: u8, T: Tracing> TownsMachine<CPU_MODEL, T> {
    /// Overrides the host local-time source (BCD) used by the RTC.
    pub fn set_host_local_time_fn(&mut self, host_local_time_fn: fn() -> [u8; 6]) {
        self.bus.set_host_local_time_fn(host_local_time_fn);
    }

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
        self.bus.insert_hdd(drive, image, path);
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
    fn run_for_impl(&mut self, budget: u64) -> u64 {
        let start = self.bus.current_cycle;
        let target = start + budget;
        while self.bus.current_cycle < target {
            let current = self.bus.current_cycle;
            let slice_end = if self.cpu.halted() {
                let next = self.bus.next_event_cycle().unwrap_or(target);
                next.clamp(current + 1, target)
            } else {
                (current + TIGHT_SLICE).min(target)
            };

            let ran = self.cpu.run_for(slice_end - current, &mut self.bus);

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
            if ran == 0 && self.bus.current_cycle < slice_end {
                self.bus.set_current_cycle(slice_end);
            }
        }
        self.bus.current_cycle - start
    }
}

impl<const CPU_MODEL: u8, T: Tracing> Machine for TownsMachine<CPU_MODEL, T> {
    fn cpu_clock_hz(&self) -> f64 {
        f64::from(self.bus.clocks.cpu_clock_hz)
    }

    fn run_for(&mut self, budget: u64) -> u64 {
        self.run_for_impl(budget)
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

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let description = insert_floppy_impl(&mut self.bus, drive, path)?;
        self.refresh_boot_device();
        Ok(description)
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
fn insert_floppy_impl<T: Tracing>(
    bus: &mut TownsBus<T>,
    drive: usize,
    path: &std::path::Path,
) -> Result<String, String> {
    let data = std::fs::read(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let image = device::floppy::load_floppy_image(path, &data)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let description = format!("{} ({})", image.name, image.format_name());
    bus.insert_floppy(drive, image, path.to_path_buf());
    Ok(description)
}

/// Loads a CD-ROM disc image (`.cue` or `.ccd`) and inserts it into the bus,
/// returning a short description. Mirrors the PC-9801 machine's loader.
fn insert_cdrom_impl<T: Tracing>(
    bus: &mut TownsBus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());

    if extension.as_deref() == Some("ccd") {
        insert_cdrom_ccd(bus, path)
    } else {
        insert_cdrom_cue(bus, path)
    }
}

fn insert_cdrom_cue<T: Tracing>(
    bus: &mut TownsBus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let cue_content = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let bin_filenames = device::cdrom::extract_bin_filenames(&cue_content)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let base_path = path.parent().unwrap_or(std::path::Path::new("."));
    let mut bin_files = Vec::with_capacity(bin_filenames.len());
    for bin_filename in &bin_filenames {
        let bin_path = base_path.join(bin_filename);
        let bin_data = std::fs::read(&bin_path)
            .map_err(|error| format!("Failed to read {}: {error}", bin_path.display()))?;
        bin_files.push(bin_data);
    }
    let image = device::cdrom::CdImage::from_cue_files(&cue_content, bin_files)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let description = format!(
        "{} ({} tracks, {} sectors)",
        bin_filenames[0],
        image.track_count(),
        image.total_sectors()
    );
    bus.insert_cdrom(image);
    Ok(description)
}

fn insert_cdrom_ccd<T: Tracing>(
    bus: &mut TownsBus<T>,
    path: &std::path::Path,
) -> Result<String, String> {
    let ccd_content = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let img_path = path.with_extension("img");
    let img_data = std::fs::read(&img_path)
        .map_err(|error| format!("Failed to read {}: {error}", img_path.display()))?;
    let sub_path = path.with_extension("sub");
    let sub_data = match std::fs::read(&sub_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!("Failed to read {}: {error}", sub_path.display()));
        }
    };
    let has_sub = sub_data.is_some();
    let image = device::cdrom::CdImage::from_ccd(&ccd_content, img_data, sub_data)
        .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
    let img_name = img_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image.img");
    let description = format!(
        "{} ({} tracks, {} sectors, {})",
        img_name,
        image.track_count(),
        image.total_sectors(),
        if has_sub { "CCD+SUB" } else { "CCD" }
    );
    bus.insert_cdrom(image);
    Ok(description)
}
