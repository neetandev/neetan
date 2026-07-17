//! PC-88VA2 machine: a NEC V30 main CPU and a Z80 floppy sub-CPU.
//!
//! A single monotonic `current_cycle` (main-clock units) drives the scheduler;
//! the sub CPU runs proportional T-states converted by the clock ratio. The
//! slice is kept tight, and tighter still while a PPI mailbox handshake is in
//! flight, so the two cores make progress together.

use common::{Bus, Cpu, CpuZ80, NoTrace, TraceSink};

use crate::bus::{Pc88VaBus, SYNC_SLICE, SubBusView, TIGHT_SLICE};

save_state::runtime_state! {
/// Machine-root state for one PC-88VA snapshot.
#[derive(Clone)]
struct Pc88VaRuntimeState {
    main_cpu: cpu::V30State,
    sub_cpu: cpu::Z80State,
    bus: crate::bus::Pc88VaBusState,
}}

const RESET_CS: u16 = 0xF000;
const RESET_IP: u16 = 0xFFF0;

/// A PC-88VA2 machine: the V30 main CPU, the Z80 floppy sub-CPU, and the VA bus.
pub struct Pc88VaMachine<T: TraceSink = NoTrace> {
    /// The V30 main CPU.
    pub cpu: cpu::V30,
    /// The Z80 floppy sub-CPU (PC80S31K).
    pub sub_cpu: cpu::Z80,
    /// The VA system bus, owning memory and devices.
    pub bus: Pc88VaBus<T>,
}

/// Builds an untraced PC-88VA machine around a configured bus.
pub fn build_untraced_machine(bus: Pc88VaBus<NoTrace>) -> Box<dyn common::Machine> {
    let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
    Box::new(Pc88VaMachine::new(reset_cpu(), sub_cpu, bus))
}

impl<T: TraceSink> Pc88VaMachine<T> {
    /// Builds a machine around configured CPUs and bus.
    pub fn new(cpu: cpu::V30, sub_cpu: cpu::Z80, bus: Pc88VaBus<T>) -> Self {
        Self { cpu, sub_cpu, bus }
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
        let start_cycle = self.bus.current_cycle();
        if T::ENABLED && self.bus.tracer().yield_requested() {
            return 0;
        }
        if T::ENABLED {
            self.run_sub_for_main_cycles(0);
            if self.bus.tracer().yield_requested() {
                return 0;
            }
        }
        let target_cycle = start_cycle.saturating_add(budget);

        while self.bus.current_cycle() < target_cycle {
            let current_cycle = self.bus.current_cycle();
            let resynchronizing = current_cycle < self.bus.resync_until;

            // When the V30 is halted and no handshake is in flight, fast-forward
            // to the next scheduled event so an IRQ can wake it, instead of
            // stepping one tiny slice at a time. The sub CPU still runs for the
            // elapsed time below.
            let slice_cycles = if self.cpu.halted() && !resynchronizing {
                let next_event_cycle = self.bus.next_event_cycle().unwrap_or(target_cycle);
                next_event_cycle.clamp(current_cycle + 1, target_cycle) - current_cycle
            } else {
                let slice_cap = if resynchronizing {
                    SYNC_SLICE
                } else {
                    TIGHT_SLICE
                };
                (target_cycle - current_cycle).min(slice_cap).max(1)
            };
            let slice_end = current_cycle + slice_cycles;

            self.run_main_until(slice_end);
            let elapsed_cycles = self.bus.current_cycle() - current_cycle;
            if T::ENABLED && self.bus.tracer().yield_requested() {
                self.bus.sub_clock_credit =
                    self.bus.sub_clock_credit.saturating_add(elapsed_cycles);
                break;
            }

            // Run the sub CPU for the same elapsed wall-clock, converted to its
            // T-state domain by the clock ratio.
            self.run_sub_for_main_cycles(elapsed_cycles);
            if T::ENABLED && self.bus.tracer().yield_requested() {
                break;
            }
        }

        self.bus.current_cycle() - start_cycle
    }

    /// Advances the main V30 to at least `slice_end`, idling a halted core so its
    /// scheduled events still fire and an interrupt can wake it.
    fn run_main_until(&mut self, slice_end: u64) {
        let current_cycle = self.bus.current_cycle();
        if current_cycle >= slice_end {
            return;
        }
        let ran_cycles = self.cpu.run_for(slice_end - current_cycle, &mut self.bus);

        // A halted (or fully idle) CPU consumes nothing: advance to the slice end
        // as idle so the sub CPU still runs and interrupts can wake the core.
        if ran_cycles == 0 && self.bus.current_cycle() < slice_end {
            self.bus.set_current_cycle(slice_end);
        }
    }

    /// Runs the sub CPU for `main_cycles` of elapsed main-clock time, converting to
    /// sub T-states and carrying the remainder for an exact long-run ratio.
    fn run_sub_for_main_cycles(&mut self, main_cycles: u64) {
        let shift = self.bus.sub_to_main_shift;
        let available_cycles = main_cycles + self.bus.sub_clock_credit;
        let sub_cycles = available_cycles >> shift;
        self.bus.sub_clock_credit = available_cycles - (sub_cycles << shift);
        if sub_cycles == 0 {
            return;
        }
        let mut view = SubBusView { bus: &mut self.bus };
        let ran_cycles = self.sub_cpu.run_for(sub_cycles, &mut view);
        if T::ENABLED && ran_cycles < sub_cycles {
            let remaining_cycles = (sub_cycles - ran_cycles)
                .checked_shl(shift)
                .unwrap_or(u64::MAX);
            self.bus.sub_clock_credit = self.bus.sub_clock_credit.saturating_add(remaining_cycles);
        }
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

    fn capture_machine_blob(
        &self,
    ) -> Result<save_state::MachineStateBlob, save_state::SaveStateError> {
        let root = Pc88VaRuntimeState {
            main_cpu: self.cpu.capture_state(),
            sub_cpu: self.sub_cpu.capture_state(),
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
            64 << 20,
            |machine| {
                Ok(Pc88VaRuntimeState {
                    main_cpu: machine.cpu.capture_state(),
                    sub_cpu: machine.sub_cpu.capture_state(),
                    bus: machine.bus.capture_runtime_state()?,
                })
            },
            |machine, state| {
                machine.cpu.restore_state(state.main_cpu)?;
                machine.sub_cpu.restore_state(state.sub_cpu)?;
                machine.bus.restore_runtime_state(state.bus)
            },
        )
    }
}

impl Pc88VaMachine<NoTrace> {
    /// Builds a reset V30 for the PC-88VA reset vector.
    pub fn reset_cpu() -> cpu::V30 {
        reset_cpu()
    }
}

/// Builds a reset V30 for the PC-88VA reset vector.
pub fn reset_cpu() -> cpu::V30 {
    let mut cpu = cpu::V30::new();
    cpu.reset();
    cpu.set_ip(RESET_IP);
    cpu.set_cs(RESET_CS);
    cpu
}

impl<T: TraceSink> common::Machine for Pc88VaMachine<T> {
    fn capture_state(&mut self) -> Result<common::MachineStateBlob, common::SaveStateError> {
        self.capture_machine_blob()
    }

    fn restore_state(
        &mut self,
        blob: &common::MachineStateBlob,
    ) -> Result<(), common::SaveStateError> {
        self.restore_machine_blob(blob)
    }

    fn set_host_date_time_provider(&mut self, provider: common::HostDateTimeProvider) {
        self.bus.set_host_date_time_provider(provider);
    }

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

#[cfg(test)]
mod tests {
    use common::{TraceAccessKind, TraceEvent, TraceSink};

    use super::{Pc88VaMachine, reset_cpu};
    use crate::{bus::Pc88VaBus, config::Pc88VaModel, rom::LoadedRoms};

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
        let model = Pc88VaModel::PC88VA2;
        let roms = LoadedRoms {
            rom00: vec![0; 0x8_0000],
            rom08: vec![0; 0x2_0000],
            rom1: vec![0; 0x2_0000],
            font: vec![0; 0x5_0000],
            dictionary: vec![0; 0x8_0000],
            subsys: vec![0; 0x2000],
        };
        let bus = Pc88VaBus::new_with_trace_sink(model, roms, 48_000, YieldOnScheduled::default());
        let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
        let mut machine = Pc88VaMachine::new(reset_cpu(), sub_cpu, bus);

        machine.run_for(100_000);

        assert!(machine.bus.tracer().saw_scheduled);
        assert!(!machine.bus.tracer().fetch_after_scheduled);
    }

    #[test]
    fn main_trace_yield_preserves_sub_cpu_clock_debt() {
        let model = Pc88VaModel::PC88VA2;
        let roms = LoadedRoms {
            rom00: vec![0; 0x8_0000],
            rom08: vec![0; 0x2_0000],
            rom1: vec![0; 0x2_0000],
            font: vec![0; 0x5_0000],
            dictionary: vec![0; 0x8_0000],
            subsys: vec![0; 0x2000],
        };
        let bus = Pc88VaBus::new_with_trace_sink(model, roms, 48_000, YieldOnMainFetch::default());
        let sub_cpu = cpu::Z80::new(bus.clock_config().sub_clock_hz);
        let mut machine = Pc88VaMachine::new(reset_cpu(), sub_cpu, bus);
        machine.bus.tracer_mut().arm();

        machine.run_for(100);

        let shift = machine.bus.sub_to_main_shift;
        let pending_tstates = machine.bus.sub_clock_credit >> shift;
        let sub_cycle_before_resume = machine.bus.sub_cycle;
        assert!(pending_tstates > 0);

        machine.bus.tracer_mut().resume();
        assert_eq!(machine.run_for(0), 0);

        assert!(machine.bus.sub_cycle >= sub_cycle_before_resume + pending_tstates);
        assert!(machine.bus.sub_clock_credit < 1 << shift);
    }
}
