//! FM-7 machine.
//!
//! The main and display MC6809 CPUs share the machine bus and are interleaved
//! according to their respective clock rates.

use common::{Cpu6809, NoTracing, Tracing};

use crate::bus::{Fm7Bus, MainBusView, SubBusView};

/// Main-clock cycles per interleave slice during normal execution.
const DEFAULT_SLICE_CYCLES: u64 = 16;
/// Main-clock cycles per interleave slice while a main/sub handshake is active.
const HANDSHAKE_SLICE_CYCLES: u64 = 4;

/// FM-7 / FM-77AV machine.
pub struct Fm7Machine<T: Tracing = NoTracing> {
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

impl<T: Tracing> Fm7Machine<T> {
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
        let start = self.bus.current_cycle();
        let target = start + budget;

        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let next = self.bus.next_event_cycle().unwrap_or(target).min(target);
            let cap = if self.bus.handshake_active() {
                HANDSHAKE_SLICE_CYCLES
            } else {
                DEFAULT_SLICE_CYCLES
            };
            let slice_end = current.saturating_add(cap).min(next);

            self.sync_main_firq();
            let slice = slice_end.saturating_sub(current).max(1);
            let ran = {
                let mut view = MainBusView { bus: &mut self.bus };
                self.main_cpu.run_for(slice, &mut view)
            };
            if ran == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }

            if self.bus.take_clock_reanchor() {
                self.sub_cycle_target = self.bus.sub_cycle();
            }
            if self.bus.take_sub_reset() {
                self.reset_sub_cpu();
            }

            let elapsed = self.bus.current_cycle().saturating_sub(current);
            self.run_sub_for_main_units(elapsed);

            if self.bus.current_cycle() >= next {
                self.bus.process_events();
            }
        }

        self.bus.current_cycle() - start
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

    /// Runs the sub CPU up to the cycles it owes for `main_units` of main time, or
    /// idles it forward while it is halt-acknowledged.
    ///
    /// The owed cycles accumulate into `sub_cycle_target`; running whole
    /// instructions may overshoot the target in one slice, and that overshoot
    /// shrinks the next slice's budget so the long-run ratio stays exact.
    fn run_sub_for_main_units(&mut self, main_units: u64) {
        let owed = self.bus.sub_cycles_for_main_units(main_units);
        self.sub_cycle_target = self.sub_cycle_target.saturating_add(owed);
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
}

impl<T: Tracing> common::Machine for Fm7Machine<T> {
    fn set_host_date_time_provider(&mut self, provider: common::HostDateTimeProvider) {
        self.bus.set_host_date_time_provider(provider);
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

    fn insert_floppy(&mut self, drive: usize, path: &std::path::Path) -> Result<String, String> {
        let data = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let image = device::floppy::load_floppy_image(path, &data)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let description = format!("{} ({})", image.name, image.format_name());
        self.bus.insert_floppy(drive, image, path.to_path_buf());
        Ok(description)
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
            .insert_cassette(extension, &image)
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
