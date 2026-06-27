//! PC-88VA2 machine: a NEC V30 main CPU and a Z80 floppy sub-CPU.
//!
//! A single monotonic `current_cycle` (main-clock units) drives the scheduler;
//! the sub CPU runs proportional T-states converted by the clock ratio. The
//! slice is kept tight, and tighter still while a PPI mailbox handshake is in
//! flight, so the two cores make progress together.

use common::{Bus, Cpu, CpuZ80};

use crate::{
    bus::{Pc88VaBus, SYNC_SLICE, SubBusView, TIGHT_SLICE},
    config::{ClockConfig, Pc88VaModel},
    memory::Pc88VaMemory,
    rom::LoadedRoms,
};

const RESET_CS: u16 = 0xF000;
const RESET_IP: u16 = 0xFFF0;
const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// A PC-88VA2 machine: the V30 main CPU, the Z80 floppy sub-CPU, and the VA bus.
pub struct Pc88VaMachine {
    /// The V30 main CPU.
    pub cpu: cpu::V30,
    /// The Z80 floppy sub-CPU (PC80S31K).
    pub sub_cpu: cpu::Z80,
    /// The VA system bus, owning memory and devices.
    pub bus: Pc88VaBus,
}

impl Pc88VaMachine {
    /// Builds a machine for `model` from a loaded ROM set and points the V30 at
    /// its reset vector (CS=0xF000, IP=0xFFF0).
    pub fn new(model: Pc88VaModel, roms: LoadedRoms) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sub_clock_hz: model.sub_clock_hz(),
            sample_rate: DEFAULT_SAMPLE_RATE,
        };
        let subsys = roms.subsys.clone();
        let memory = Pc88VaMemory::new(model, roms);
        let mut bus = Pc88VaBus::new(memory, clocks);
        bus.load_disk_rom(&subsys);

        let mut cpu = cpu::V30::new();
        cpu.reset();
        cpu.set_ip(RESET_IP);
        cpu.set_cs(RESET_CS);

        let sub_cpu = cpu::Z80::new(clocks.sub_clock_hz);

        Self { cpu, sub_cpu, bus }
    }

    /// Overrides the host local-time source (BCD) used by the RTC.
    pub fn set_host_local_time_fn(&mut self, host_local_time_fn: fn() -> [u8; 6]) {
        self.bus.set_host_local_time_fn(host_local_time_fn);
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
        let start = self.bus.current_cycle();
        let target = start + budget;
        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let resync = current < self.bus.resync_until;

            // When the V30 is halted and no handshake is in flight, fast-forward
            // to the next scheduled event so an IRQ can wake it, instead of
            // stepping one tiny slice at a time. The sub CPU still runs for the
            // elapsed time below.
            let slice = if self.cpu.halted() && !resync {
                let next = self.bus.next_event_cycle().unwrap_or(target);
                next.clamp(current + 1, target) - current
            } else {
                let slice_cap = if resync { SYNC_SLICE } else { TIGHT_SLICE };
                (target - current).min(slice_cap).max(1)
            };
            let slice_end = current + slice;

            self.run_main_until(slice_end);

            // Run the sub CPU for the same elapsed wall-clock, converted to its
            // T-state domain by the clock ratio.
            let elapsed = self.bus.current_cycle() - current;
            self.run_sub_for_main_units(elapsed);
        }
        self.bus.current_cycle() - start
    }

    /// Advances the main V30 to at least `slice_end`, idling a halted core so its
    /// scheduled events still fire and an interrupt can wake it.
    fn run_main_until(&mut self, slice_end: u64) {
        let current = self.bus.current_cycle();
        if current >= slice_end {
            return;
        }
        let ran = self.cpu.run_for(slice_end - current, &mut self.bus);

        // A halted (or fully idle) CPU consumes nothing: advance to the slice end
        // as idle so the sub CPU still runs and interrupts can wake the core.
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

    /// Mounts a floppy image (read and parsed from `path`) into a drive.
    pub fn insert_floppy(
        &mut self,
        drive: usize,
        path: &std::path::Path,
    ) -> Result<String, String> {
        let data = std::fs::read(path)
            .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("Failed to parse {}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()));
        Ok(description)
    }

    /// Ejects the floppy from a drive.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.bus.eject_floppy(drive);
    }

    /// Flushes any dirty mounted floppies back to their source files.
    pub fn flush_floppies(&mut self) {
        self.bus.flush_floppies();
    }
}

impl common::Machine for Pc88VaMachine {
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

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        Pc88VaMachine::insert_floppy(self, drive, path)
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
