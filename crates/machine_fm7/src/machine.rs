//! FM-7 machine.
//!
//! The main and display MC6809 CPUs share the machine bus and are interleaved
//! according to their respective clock rates.

use common::{Cpu6809, NoTrace, TraceSink};

use crate::bus::{Fm7Bus, MainBusView, SubBusView};

/// Main-clock cycles per interleave slice during normal execution.
const DEFAULT_SLICE_CYCLES: u64 = 16;
/// Main-clock cycles per interleave slice while a main/sub handshake is active.
const HANDSHAKE_SLICE_CYCLES: u64 = 4;

/// FM-7 / FM-77AV machine.
pub struct Fm7Machine<T: TraceSink = NoTrace> {
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

impl<T: TraceSink> Fm7Machine<T> {
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
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        if T::ENABLED {
            if self.bus.take_clock_reanchor() {
                self.sub_cycle_target = self.bus.sub_cycle();
            }
            if self.bus.take_sub_reset() {
                self.reset_sub_cpu();
            }
            self.run_sub_to_target();
            if self.bus.tracer().yield_requested() {
                return 0;
            }
        }
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
            let trace_yield_requested = T::ENABLED && self.bus.tracer().yield_requested();
            if !trace_yield_requested && ran == 0 && self.bus.current_cycle() < slice_end {
                self.bus.set_current_cycle(slice_end);
            }

            if self.bus.take_clock_reanchor() {
                self.sub_cycle_target = self.bus.sub_cycle();
            }

            let elapsed = self.bus.current_cycle().saturating_sub(current);
            self.account_sub_for_main_units(elapsed);
            if trace_yield_requested {
                break;
            }
            if self.bus.take_sub_reset() {
                self.reset_sub_cpu();
            }
            self.run_sub_to_target();
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
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

    /// Adds the sub CPU cycles owed for elapsed main-clock time.
    fn account_sub_for_main_units(&mut self, main_units: u64) {
        let owed = self.bus.sub_cycles_for_main_units(main_units);
        self.sub_cycle_target = self.sub_cycle_target.saturating_add(owed);
    }

    /// Runs or idles the sub CPU to its accumulated cycle target.
    fn run_sub_to_target(&mut self) {
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

impl<T: TraceSink> common::Machine for Fm7Machine<T> {
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

#[cfg(test)]
mod tests {
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use super::Fm7Machine;
    use crate::{
        bus::Fm7Bus,
        config::{BootMode, Fm7Model},
        rom::LoadedRoms,
    };

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
        let model = Fm7Model::Fm7;
        let roms = LoadedRoms {
            model,
            fbasic: vec![0; 0x7C00],
            subsys_c: vec![0; 0x2800],
            kanji: None,
            boot_bas: Some(vec![0; 0x0200]),
            boot_dos: Some(vec![0; 0x0200]),
            initiate: None,
            subsys_a: None,
            subsys_b: None,
            subsyscg: None,
        };
        let mut bus = Fm7Bus::new_with_trace_sink(
            model,
            BootMode::Basic,
            48_000,
            YieldOnScheduled::default(),
        );
        bus.load_roms(&roms);
        let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
        let mut machine = Fm7Machine::new(main_cpu, sub_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn main_trace_yield_preserves_sub_cpu_cycle_target() {
        let model = Fm7Model::Fm7;
        let roms = LoadedRoms {
            model,
            fbasic: vec![0; 0x7C00],
            subsys_c: vec![0; 0x2800],
            kanji: None,
            boot_bas: Some(vec![0; 0x0200]),
            boot_dos: Some(vec![0; 0x0200]),
            initiate: None,
            subsys_a: None,
            subsys_b: None,
            subsyscg: None,
        };
        let mut bus = Fm7Bus::new_with_trace_sink(
            model,
            BootMode::Basic,
            48_000,
            YieldOnMainFetch::default(),
        );
        bus.load_roms(&roms);
        let main_cpu = cpu::M6809::new(bus.cpu_clock_hz());
        let sub_cpu = cpu::M6809::new(model.sub_clock_hz());
        let mut machine = Fm7Machine::new(main_cpu, sub_cpu, bus);
        machine.bus.tracer_mut().arm();

        machine.run_for(100);

        let pending_target = machine.sub_cycle_target;
        assert!(pending_target > machine.bus.sub_cycle());

        machine.bus.tracer_mut().resume();
        assert_eq!(machine.run_for(0), 0);

        assert!(machine.bus.sub_cycle() >= pending_target);
    }
}
