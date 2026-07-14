//! Top-level PC-6000 machine.
//!
//! A single-Z80 machine: one CPU driving one bus, paced by a monotonic
//! `current_cycle` in main-clock units.

use common::{CpuZ80, NoTracing, Tracing};

use crate::bus::{MainBusView, Pc6000Bus};

/// PC-6000 machine: the main Z80 sharing one bus.
pub struct Pc6000Machine<T: Tracing = NoTracing> {
    /// Main CPU.
    pub main_cpu: cpu::Z80,
    /// System bus.
    pub bus: Pc6000Bus<T>,
}

impl<T: Tracing> Pc6000Machine<T> {
    /// Creates a new machine from the given CPU and bus.
    pub fn new(main_cpu: cpu::Z80, bus: Pc6000Bus<T>) -> Self {
        Self { main_cpu, bus }
    }

    /// Runs the main CPU for up to `budget` main-clock cycles, returning the
    /// cycles actually advanced. Execution is sliced to the next scheduled
    /// event so timer and frame interrupts fire promptly; a halted CPU idles
    /// forward to the next event to keep the scheduler clock moving.
    pub fn run_for(&mut self, budget: u64) -> u64 {
        let start = self.bus.current_cycle();
        let target = start + budget;

        while self.bus.current_cycle() < target {
            let current = self.bus.current_cycle();
            let next = self
                .bus
                .scheduler
                .next_event_cycle()
                .unwrap_or(target)
                .min(target);

            if self.bus.cpu_stalled() {
                // The video circuit holds the bus; the CPU idles to the next
                // event (the bus-request release) without executing.
                self.bus.set_current_cycle(next);
            } else {
                let slice = next.saturating_sub(current).max(1);
                let ran = {
                    let mut view = MainBusView { bus: &mut self.bus };
                    self.main_cpu.run_for(slice, &mut view)
                };
                if ran == 0 && self.bus.current_cycle() < next {
                    self.bus.set_current_cycle(next);
                }
            }

            if self.bus.current_cycle() >= next {
                self.bus.process_events();
            }
        }

        self.bus.current_cycle() - start
    }
}

impl<T: Tracing> common::Machine for Pc6000Machine<T> {
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
        Pc6000Machine::run_for(self, budget)
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
        self.bus
            .insert_floppy(drive, image, Some(path.to_path_buf()));
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
    use common::{CpuZ80, Machine};

    use super::*;
    use crate::{config::Pc6000Model, rom::LoadedRoms};

    fn loaded_roms_with_boot(model: Pc6000Model, boot: Vec<u8>) -> LoadedRoms {
        LoadedRoms {
            model,
            basic: Some(boot),
            system_rom1: None,
            system_rom2: None,
            sub_rom: None,
            cg_base: None,
            cg_ext: None,
            cg_sr: None,
            kanji: None,
            voice: None,
        }
    }

    fn machine_with_boot(model: Pc6000Model, boot: Vec<u8>) -> Pc6000Machine {
        let mut bus = Pc6000Bus::new(model, 48_000);
        bus.load_roms(&loaded_roms_with_boot(model, boot));
        let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
        Pc6000Machine::new(main_cpu, bus)
    }

    #[test]
    fn cpu_executes_rom() {
        // A boot ROM of NOPs (0x00) lets the CPU stream forward from the reset
        // vector without trapping.
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        let start_pc = machine.main_cpu.pc();

        let ran = machine.run_for(100);

        assert!(ran >= 100, "the cycle budget is consumed");
        assert_ne!(
            machine.main_cpu.pc(),
            start_pc,
            "the CPU advances through ROM"
        );
    }

    #[test]
    fn audio_pacing_tracks_elapsed_cycles() {
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, vec![0x00; 0x1000]);
        machine.run_for(machine.bus.cpu_clock_hz() as u64);
        let mut output = vec![0.0_f32; 200_000];
        let written = machine.generate_audio_samples(1.0, &mut output);
        // One second of main-clock cycles is roughly one second of samples.
        assert!(
            (2 * 47_000..=2 * 49_000).contains(&written),
            "got {written}"
        );
    }

    #[test]
    fn boot_program_renders_a_screen() {
        // The boot program selects the 0xC000 text base. The base text background
        // is a non-black pen, so once a frame renders the framebuffer carries lit
        // pixels.
        let boot = vec![
            0xF3, // DI
            0x3E, 0x00, // LD A, 0x00
            0xD3, 0xB0, // OUT (0xB0), A   ; select 0xC000 base, text mode
            0x18, 0xFE, // JR $
        ];
        let mut machine = machine_with_boot(Pc6000Model::Pc6001, boot);

        let frame = u64::from(machine.bus.cpu_clock_hz()) / 60;
        for _ in 0..4 {
            machine.run_for(frame);
        }

        let framebuffer = machine.display_framebuffer();
        let lit = framebuffer
            .chunks_exact(4)
            .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            .count();
        assert!(lit > 0, "the boot program should render a non-blank screen");
    }

    #[test]
    fn mkii_reports_extended_display_dimensions() {
        // The mkII renders a 320x240 image.
        let machine = machine_with_boot(Pc6000Model::Pc6001Mk2, vec![0x00; 0x1000]);
        assert_eq!(machine.display_dimensions(), (320, 240));
    }
}
