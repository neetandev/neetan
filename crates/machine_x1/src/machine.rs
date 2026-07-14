//! Top-level Sharp X1 machine.
//!
//! A single-Z80 machine: one CPU driving one bus, paced by a monotonic
//! `current_cycle` in main-clock units.

use common::{CpuZ80, NoTrace, TraceSink};

use crate::bus::{MainBusView, X1Bus};

/// Sharp X1 machine: the main Z80 sharing one bus.
pub struct X1Machine<T: TraceSink = NoTrace> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// System bus.
    pub bus: X1Bus<T>,
}

impl<T: TraceSink> X1Machine<T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(main_cpu: cpu::Z80, bus: X1Bus<T>) -> Self {
        Self { main_cpu, bus }
    }

    /// Runs the main CPU for up to `budget` main-clock cycles, returning the
    /// cycles actually advanced. Execution is sliced to the next scheduled event
    /// so periodic interrupts fire promptly.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        let target = start + budget;

        // Bound continuous-mode DMA stalls to this run so a long floppy transfer
        // is sliced across audio steps rather than overrunning one in a single
        // instruction.
        self.bus.set_dma_stall_deadline(target);

        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let next = self
                .bus
                .scheduler
                .next_event_cycle()
                .unwrap_or(target)
                .min(target);

            let slice = next.saturating_sub(current).max(1);
            let ran = {
                let mut view = MainBusView { bus: &mut self.bus };
                self.main_cpu.run_for(slice, &mut view)
            };
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
            if ran == 0 && self.bus.current_cycle() < next {
                self.bus.set_current_cycle(next);
            }

            if self.bus.current_cycle() >= next {
                self.bus.process_events();
                if T::ENABLED && self.bus.tracer().yield_requested() {
                    break;
                }
            }
        }

        self.bus.current_cycle() - start
    }
}

impl<T: TraceSink> common::Machine for X1Machine<T> {
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
        X1Machine::run_for(self, budget)
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

#[cfg(test)]
mod tests {
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use super::X1Machine;
    use crate::{bus::X1Bus, config::X1Model, rom::LoadedRoms};

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

    #[test]
    fn scheduled_trace_yield_prevents_a_later_fetch() {
        let model = X1Model::X1;
        let roms = LoadedRoms {
            model,
            ipl: vec![0; model.ipl_rom_size()],
            cgrom_8x8: vec![0; 0x0800],
            ank: vec![0; 0x2000],
            kanji: None,
        };
        let mut bus = X1Bus::new_with_trace_sink(model, 48_000, YieldOnScheduled::default());
        bus.load_roms(&roms);
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        let mut machine = X1Machine::new(main_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }
}
