//! Z80 DMA glue for the X1 turbo.
//!
//! The controller runs in single mode: the bus drives [`device::z80_dma::Z80Dma::do_dma`]
//! once after every CPU instruction (and from the [`EventX1::DmaTick`] pacing
//! event while the CPU is halted or absent). DMA memory accesses go to main
//! RAM; DMA I/O accesses go through the full port decode, so a transfer can
//! stream floppy sector bytes from the FDC data register straight into the
//! I/O-mapped bitmap VRAM. Bus clocks the controller consumes are charged to
//! the CPU as wait cycles.

use common::Tracing;
use device::z80_dma::Z80DmaBus;

use super::X1Bus;
use crate::scheduler::EventX1;

/// Ephemeral [`Z80DmaBus`] view over the X1 bus for one transfer run.
struct X1DmaBusAdapter<'a, T: Tracing> {
    bus: &'a mut X1Bus<T>,
    /// Wait cycles the last memory/io access incurred, pulled back out of the
    /// bus's wait accumulator so the controller decides whether to honor them.
    access_wait: u32,
    /// Cycle past which a continuous/burst grant stops transferring (via
    /// [`Z80DmaBus::may_continue_transfer`]), so it pauses at the current run's
    /// budget instead of stalling the CPU through an entire multi-sector block
    /// in one instruction.
    deadline: u64,
}

impl<T: Tracing> X1DmaBusAdapter<'_, T> {
    /// The current cycle including wait cycles pending from this instruction
    /// boundary, so ready sampling sees the stolen time.
    fn effective_cycle(&self) -> u64 {
        self.bus
            .current_cycle
            .wrapping_add(self.bus.wait_cycles.max(0) as u64)
    }
}

impl<T: Tracing> Z80DmaBus for X1DmaBusAdapter<'_, T> {
    fn read_memory(&mut self, address: u16) -> u8 {
        self.bus.memory.read(address)
    }

    fn write_memory(&mut self, address: u16, value: u8) {
        self.bus.memory.write(address, value);
    }

    fn read_io(&mut self, port: u16) -> u8 {
        let waits_before = self.bus.wait_cycles;
        let value = self.bus.io_read(port);
        let delta = (self.bus.wait_cycles - waits_before).max(0);
        self.bus.wait_cycles -= delta;
        self.access_wait = delta as u32;
        value
    }

    fn write_io(&mut self, port: u16, value: u8) {
        let waits_before = self.bus.wait_cycles;
        self.bus.io_write(port, value);
        let delta = (self.bus.wait_cycles - waits_before).max(0);
        self.bus.wait_cycles -= delta;
        self.access_wait = delta as u32;
    }

    fn ready_line(&mut self) -> bool {
        let now = self.effective_cycle();
        self.bus.fdc.drq_line(now)
    }

    fn add_cpu_clock(&mut self, cycles: u32) {
        self.bus.add_wait_cycles(i64::from(cycles));
    }

    fn take_access_wait(&mut self) -> u32 {
        core::mem::take(&mut self.access_wait)
    }

    fn may_continue_transfer(&self) -> bool {
        // Pause a continuous/burst grant once it has stalled the CPU up to this
        // run's budget. Unlike the ready line, this is honored even under a
        // force-ready grant, so a large block (which ignores FDC flow control)
        // is still sliced across runs. The remaining bytes transfer on the next
        // instruction / DmaTick; throughput and per-byte timing are unchanged.
        self.effective_cycle() < self.deadline
    }
}

impl<T: Tracing> X1Bus<T> {
    /// Runs the DMA transfer engine once (single-mode DMA). While a
    /// continuous-mode transfer holds the bus waiting for the ready line, the
    /// stall is charged to the CPU up to the next data-request slot, but never
    /// past [`X1Bus::dma_stall_deadline`] (the current run's budget) so a long
    /// transfer is sliced across runs.
    pub(crate) fn do_dma(&mut self) {
        if !self.model.has_dma() {
            return;
        }
        let deadline = self.dma_stall_deadline;
        let mut dma = core::mem::take(&mut self.dma);
        let mut adapter = X1DmaBusAdapter {
            bus: self,
            access_wait: 0,
            deadline,
        };
        loop {
            dma.do_dma(&mut adapter);
            if !(dma.holds_bus() && dma.is_enabled()) {
                break;
            }
            let Some(next_request) = adapter.bus.fdc.next_drq_cycle() else {
                break;
            };
            let now = adapter.effective_cycle();
            if next_request <= now {
                break;
            }
            // The transfer holds the bus but the next byte is not ready yet;
            // stop if that wait would run past this run's budget (deferred to the
            // next slice), otherwise charge the inter-request gap to the CPU.
            if now >= deadline {
                break;
            }
            let gap = (next_request - now) as i64;
            let room = (deadline - now) as i64;
            if gap <= room {
                adapter.bus.add_wait_cycles(gap);
            } else {
                adapter.bus.add_wait_cycles(room);
                break;
            }
        }
        self.dma = dma;
        self.sync_interrupts();
        self.sync_dma_tick();
    }

    /// Keeps a pacing event scheduled at the next FDC data-request slot while
    /// the controller is armed, so transfers progress when the CPU is halted
    /// and in bus-driven tests without a CPU.
    pub(crate) fn sync_dma_tick(&mut self) {
        if !self.model.has_dma() {
            return;
        }
        match self.fdc.next_drq_cycle() {
            Some(cycle) if self.dma.is_enabled() => {
                let fire_cycle = cycle.max(self.current_cycle.saturating_add(1));
                self.scheduler.schedule(EventX1::DmaTick, fire_cycle);
            }
            _ => self.scheduler.cancel(EventX1::DmaTick),
        }
    }

    /// Refreshes the controller's sampled ready-line level from the FDC
    /// data-request line (level-sensed, checked on control accesses).
    pub(crate) fn refresh_dma_ready_line(&mut self) {
        let level = self.fdc.drq_line(self.current_cycle);
        self.dma.set_ready_line(level);
    }
}
