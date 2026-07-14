//! Top-level PC-8801 machine.
//!
//! Wires the main Z80 and the disk sub-CPU (PC80S31K) to the shared bus and
//! interleaves them for a cycle budget. A single monotonic `current_cycle` (in
//! main-clock units) drives the scheduler; the sub CPU runs proportional
//! T-states converted by the clock ratio. The slice is kept tight, and tighter
//! still while a PPI mailbox handshake is in flight, so the two cores make
//! progress together.

use common::{CpuZ80, NoTracing, Tracing};

use crate::bus::{MainBusView, Pc8801Bus, SYNC_SLICE, SubBusView, TIGHT_SLICE};

/// PC-8801 machine: the main Z80 and the disk sub-CPU sharing one bus.
pub struct Pc8801Machine<T: Tracing = NoTracing> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// Disk sub-CPU (PC80S31K).
    pub sub_cpu: cpu::Z80,
    /// System bus.
    pub bus: Pc8801Bus<T>,
}

impl<T: Tracing> Pc8801Machine<T> {
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
        let start = self.bus.current_cycle();
        let target = start + budget;
        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let slice_cap = if current < self.bus.resync_until {
                SYNC_SLICE
            } else {
                TIGHT_SLICE
            };
            let slice = (target - current).min(slice_cap).max(1);
            let slice_end = current + slice;

            self.run_main_until(slice_end);

            // Run the sub CPU for the same elapsed wall-clock, converted to its
            // T-state domain by the clock ratio.
            let elapsed = self.bus.current_cycle() - current;
            self.run_sub_for_main_units(elapsed);
        }
        self.bus.current_cycle() - start
    }

    /// Advances the main CPU to at least `slice_end`, honoring the V1S/N text-DMA
    /// BUSREQ lockout and guaranteeing forward progress when the CPU is halted.
    fn run_main_until(&mut self, slice_end: u64) {
        let current = self.bus.current_cycle();
        if current >= slice_end {
            return;
        }

        // The text DMA holds the bus (V1S/N display lockout): the main CPU cannot
        // execute, so advance time so its scheduled events still fire.
        if self.bus.busreq_active() {
            let until = self.bus.busreq_until().min(slice_end).max(current + 1);
            self.bus.set_current_cycle(until);
            return;
        }

        let ran = {
            let mut view = MainBusView { bus: &mut self.bus };
            self.main_cpu.run_for(slice_end - current, &mut view)
        };

        // A halted (or fully waited) CPU consumes nothing: advance to the slice
        // end as idle so the sub CPU still runs and interrupts can wake the core.
        if ran == 0 && self.bus.current_cycle() < slice_end {
            self.bus.set_current_cycle(slice_end);
        }
    }

    /// Runs the sub CPU for `main_units` of elapsed main-clock time, converting to
    /// sub T-states and carrying the remainder for an exact long-run ratio.
    fn run_sub_for_main_units(&mut self, main_units: u64) {
        if main_units == 0 {
            return;
        }
        let shift = self.bus.sub_to_main_shift;
        let available = main_units + self.bus.sub_clock_credit;
        let tstates = available >> shift;
        self.bus.sub_clock_credit = available - (tstates << shift);
        if tstates == 0 {
            return;
        }
        let mut view = SubBusView { bus: &mut self.bus };
        self.sub_cpu.run_for(tstates, &mut view);
    }
}

impl<T: Tracing> common::Machine for Pc8801Machine<T> {
    fn set_host_date_time_provider(&mut self, provider: common::HostDateTimeProvider) {
        self.bus.set_host_date_time_provider(provider);
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

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()));
        Ok(description)
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

fn insert_cdrom_impl<T: Tracing>(
    bus: &mut Pc8801Bus<T>,
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
    bus: &mut Pc8801Bus<T>,
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
    bus: &mut Pc8801Bus<T>,
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

#[cfg(test)]
mod tests {
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
        assert_eq!(machine.bus.io_read(0x03), 0b1111_1011);

        // Bit 7 set marks a release.
        machine.push_keyboard_scancode(0x1A | 0x80);
        assert_eq!(machine.bus.io_read(0x03), 0xFF);
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
}
