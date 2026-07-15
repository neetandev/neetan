//! PC-88VA2 system bus.
//!
//! Owns the memory map and all main-CPU peripherals, and dispatches the V30's
//! memory and I/O accesses. The VA decodes the full 16-bit I/O port address, so
//! the dispatch matches on the whole port value.

mod cgrom;
mod init;
mod io_read;
mod io_write;
mod keyboard;
mod main_fdc;
mod sgp;
mod sound;
mod sub_fdc;
mod sub_io_read;
mod sub_io_write;

use common::{
    Bus, HostDateTimeProvider, NoTrace, TraceAccessKind, TraceAccessWidth, TraceAddressSpace,
    TraceContext, TraceEvent, TraceInterruptAction, TraceInterruptKind, TracePresentation,
    TraceSink, trace_id,
};
use device::{
    cgrom_pc88va::CgromVa,
    graphics_access_pc88va::GraphicsAccessVa,
    i8253_pit::I8253Pit,
    i8259a_pic::I8259aPic,
    keyboard_pc88va::KeyboardVa,
    mouse_pc88va::MouseVa,
    pc80s31k::{Pc80s31kMemory, Pc80s31kPpiLink},
    soundboard_ii::SoundboardII,
    system_port_pc88va::SysPortVa,
    tsp_pc88va::{FramePhase, HsyncMode, Sysp4Phase, TspMemEffect, TspState},
    upd765a_fdc::{FloppyController, UPD765_PLATFORM_STANDARD, Upd765aFdc},
    upd4990a_rtc::Upd4990aRtc,
    upd71071_dma::Upd71071Dma,
    video_pc88va::VideoVa,
};
use sgp::SgpState;
use software_renderer::va::{HsyncModeVa, RenderInputsVa, VaRenderer};

use crate::{
    config::{ClockConfig, Pc88VaModel},
    memory::Pc88VaMemory,
    rom::LoadedRoms,
    scheduler::{Event88Va, Pc88VaScheduler},
};

const OPEN_BUS: u8 = 0xFF;

/// Joystick direction bits on OPN port A (register 0x0E), active low.
const JOYSTICK_UP: u8 = 0x01;
const JOYSTICK_DOWN: u8 = 0x02;
const JOYSTICK_LEFT: u8 = 0x04;
const JOYSTICK_RIGHT: u8 = 0x08;
/// Joystick trigger bits on OPN port B (register 0x0F), active low.
const JOYSTICK_TRIGGER1: u8 = 0x01;
const JOYSTICK_TRIGGER2: u8 = 0x02;

/// PIT input clock divisor relative to the main CPU clock. The 8253 base input
/// is `main_clock_hz / 4`; the uPD9002 selector (port 0xFFF0) divides further.
const PIT_BASE_DIVISOR: u32 = 4;

/// Slice clamp (main-clock units) while a PPI handshake is in flight: about one
/// sub-CPU instruction.
pub(crate) const SYNC_SLICE: u64 = 4;
/// Default fine interleave slice (main-clock units) when the link is idle.
pub(crate) const TIGHT_SLICE: u64 = 16;

/// The PC-88VA2 system bus seen by the V30.
pub struct Pc88VaBus<T: TraceSink = NoTrace> {
    tracer: T,
    pub(crate) memory: Pc88VaMemory,
    pub(crate) clocks: ClockConfig,
    pub(crate) current_cycle: u64,
    pub(crate) next_event_cycle: u64,
    pub(crate) scheduler: Pc88VaScheduler,
    /// Master/slave 8259 PIC (master at 0x188/0x18A, slave at 0x184/0x186).
    pub(crate) pic: I8259aPic,
    /// 8253 interval timer (channels at 0x1A0/0x1A2/0x1A4, control at 0x1A6).
    pub(crate) pit: I8253Pit,
    /// uPD4990A real-time clock, strobed from sysport 0x010 and 0x040.
    pub(crate) rtc: Upd4990aRtc,
    /// System ports / calendar / DIP-switch state.
    pub(crate) sysport: SysPortVa,
    /// Text and Sprite Processor: display timing and the VSYNC loop.
    pub(crate) tsp: TspState,
    /// Video controller register file and palette.
    pub(crate) video: VideoVa,
    /// Graphics access controller: the CPU's GVRAM access path.
    pub(crate) gactrlva: GraphicsAccessVa,
    /// Super Graphic Processor (SGP): the blitter coprocessor.
    pub(crate) sgp: SgpState,
    /// Sound board 2: the YM2608 (OPNA) FM/SSG/ADPCM device.
    pub(crate) soundboard: SoundboardII,
    /// Bus mouse, read through the OPNA SSG I/O ports.
    pub(crate) mouse: MouseVa,
    /// Joystick direction lines for the OPN port A read (register 0x0E), active
    /// low: bit0 up, bit1 down, bit2 left, bit3 right; bits 7:4 = 1.
    pub(crate) joystick_port_a: u8,
    /// Joystick trigger lines for the OPN port B read (register 0x0F), active
    /// low: bit0 trigger1, bit1 trigger2; bits 7:2 = 1.
    pub(crate) joystick_port_b: u8,
    /// Which device the shared mouse/joystick port (SSG 0x0E/0x0F) reports. The
    /// VA connects either a mouse or a joypad at a time, not both; this picks which,
    /// switched by whichever device last got input.
    pub(crate) joystick_selected: bool,
    /// HLE keyboard: a scancode FIFO raising master IRQ1.
    pub(crate) keyboard: KeyboardVa,
    /// Kanji / character-generator ROM access window.
    pub(crate) cgrom: CgromVa,
    /// CPU-side display renderer producing the RGBA framebuffer.
    pub(crate) renderer: VaRenderer,
    /// Valid framebuffer width from the last rendered frame.
    pub(crate) display_width: u32,
    /// Valid framebuffer height from the last rendered frame.
    pub(crate) display_height: u32,
    /// Number assigned to the next published frame.
    pub(crate) presented_frames: u64,
    /// Host BCD local-time source used by the RTC's TIME_READ command.
    pub(crate) host_date_time_provider: HostDateTimeProvider,
    /// Floppy sub-CPU (PC80S31K) 64 KiB memory: ROM, init pattern, and RAM.
    pub(crate) sub_mem: Pc80s31kMemory,
    /// Sub-CPU T-state position, tracked separately from `current_cycle`.
    pub(crate) sub_cycle: u64,
    /// Right-shift converting elapsed main-clock units to sub-CPU T-states.
    pub(crate) sub_to_main_shift: u32,
    /// Fractional main-unit remainder carried for an exact long-run clock ratio.
    pub(crate) sub_clock_credit: u64,
    /// uPD765A floppy disk controller, driven by the sub-CPU via PIO.
    pub(crate) fdc: Upd765aFdc<UPD765_PLATFORM_STANDARD>,
    /// Mounted floppy images.
    pub(crate) floppy: FloppyController,
    /// Linked main and disk-side PPI mailbox.
    pub(crate) ppi_link: Pc80s31kPpiLink,
    /// Per-drive density select latch (sub I/O 0xF4).
    pub(crate) drive_mode: u8,
    /// Motor-control latch (sub I/O 0xF8).
    pub(crate) motor_on: u8,
    /// Whether the FDC terminal-count line is currently asserted.
    pub(crate) tc_active: bool,
    /// Main-clock cycle until which the CPUs interleave tightly for a handshake.
    pub(crate) resync_until: u64,
    /// PIO data-rate pacing: main-clock units between DRQ byte slots.
    pub(crate) drq_byte_cycles: u64,
    /// FDC operating mode for the direct main-CPU path (port 0x1B0 bit 0):
    /// `true` selects DMA mode, which routes the uPD765A interrupt to the main
    /// 8259 (slave IR3, IRQ 11) instead of the floppy sub-CPU.
    pub(crate) fdc_dma_mode: bool,
    /// uPD71071 DMA controller (channel 2 serves the main-CPU FDC path).
    pub(crate) dmac: Upd71071Dma,
    /// General-purpose timer 3 (TCU) control latch (port 0x1A8): bit 7 MINTEN
    /// enables the periodic slave IRQ 13, bits 0-1 select the 120/60/30/15 Hz rate.
    pub(crate) timer3_ctrl: u8,
}

impl Pc88VaBus<NoTrace> {
    /// Builds an untraced bus for a model from its validated ROM set.
    pub fn new(model: Pc88VaModel, roms: LoadedRoms, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(model, roms, sample_rate, NoTrace)
    }
}

impl<T: TraceSink> Pc88VaBus<T> {
    /// Arms tight interleave after a PC80S31K mailbox handshake change.
    pub(crate) fn arm_ppi_resync(&mut self) {
        self.resync_until = self.current_cycle + SYNC_SLICE;
    }

    /// Builds a bus with an explicitly supplied trace sink.
    pub fn new_with_trace_sink(
        model: Pc88VaModel,
        roms: LoadedRoms,
        sample_rate: u32,
        tracer: T,
    ) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sub_clock_hz: model.sub_clock_hz(),
            sample_rate,
        };
        let subsys = roms.subsys.clone();
        let mut bus =
            Self::from_parts_with_trace_sink(Pc88VaMemory::new(model, roms), clocks, tracer);
        bus.load_disk_rom(&subsys);
        bus
    }

    /// Returns a reference to the trace sink.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Returns a mutable reference to the trace sink.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// The machine's clock configuration.
    pub fn clock_config(&self) -> ClockConfig {
        self.clocks
    }

    /// Overrides the host local-time source (BCD), used by tests.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
    }

    /// The cycle of the next scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// The current effective PIT input clock in Hz, after the uPD9002 divider.
    fn pit_clock_hz(&self) -> u32 {
        let base = self.clocks.main_clock_hz / PIT_BASE_DIVISOR;
        base >> (self.memory.upd9002_tcks() & 3)
    }

    /// Schedules timer 0's next terminal count from the current cycle.
    pub(crate) fn schedule_pit_timer0(&mut self) {
        let period = self
            .pit
            .timer0_period_cycles(self.clocks.main_clock_hz, self.pit_clock_hz());
        self.scheduler
            .schedule(Event88Va::PitTimer0, self.current_cycle + period);
    }

    pub(crate) fn update_next_event_cycle(&mut self) {
        self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
    }

    /// Applies a write to the general-purpose timer 3 control port (0x1A8). When
    /// MINTEN (bit 7) is set the periodic IRQ 13 timer runs at 120/60/30/15 Hz
    /// (bits 0-1); clearing it stops the timer and drops the line.
    pub(crate) fn write_timer3_ctrl(&mut self, value: u8) {
        self.timer3_ctrl = value;
        if value & 0x80 != 0 {
            self.schedule_timer3();
        } else {
            self.scheduler.cancel(Event88Va::Timer3);
            self.pic.clear_irq(13);
        }
        self.update_next_event_cycle();
    }

    /// Schedules the next timer 3 tick from the current cycle at the rate
    /// selected by control bits 0-1 (120 Hz >> n).
    fn schedule_timer3(&mut self) {
        let divider = 1u32 << (self.timer3_ctrl & 0x03);
        let frequency = 120 / divider;
        let period = u64::from(self.clocks.main_clock_hz / frequency);
        self.scheduler
            .schedule(Event88Va::Timer3, self.current_cycle + period);
    }

    fn process_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        for event in &due {
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::scheduler_main(
                        self.current_cycle,
                        Some(u64::from(self.clocks.main_clock_hz)),
                    ),
                    TraceEvent::Scheduled {
                        event: event.kind.trace_name(),
                        fire_tick: event.fire_cycle,
                    },
                );
            }
            match event.kind {
                Event88Va::PitTimer0 => {
                    if self.pit.advance_timer0(self.current_cycle) {
                        self.pic.set_irq(0);
                    }
                    self.schedule_pit_timer0();
                }
                Event88Va::TspFrame => self.on_tsp_frame(),
                Event88Va::Sysp4Vsync => self.on_sysp4_vsync(),
                Event88Va::SgpComplete => self.on_sgp_complete(),
                Event88Va::FdcDrqByte => self.on_fdc_drq_byte(event.fire_cycle),
                Event88Va::FdcSeekComplete => {
                    self.fdc.state.interrupt_pending = true;
                    self.update_main_fdc_irq();
                }
                Event88Va::FdcResultComplete => {
                    self.fdc.state.interrupt_pending = true;
                    self.update_main_fdc_irq();
                }
                Event88Va::FdcTcClear => {
                    self.tc_active = false;
                    self.fdc.state.tc = false;
                }
                Event88Va::OpnaTimerA => {
                    self.soundboard.timer_expired(0, self.current_cycle);
                    self.apply_sound_timers();
                }
                Event88Va::OpnaTimerB => {
                    self.soundboard.timer_expired(1, self.current_cycle);
                    self.apply_sound_timers();
                }
                Event88Va::Timer3 => {
                    if self.timer3_ctrl & 0x80 != 0 {
                        self.pic.set_irq(13);
                        self.schedule_timer3();
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    /// The horizontal sync mode implied by the current CRT configuration, used
    /// for the TSP timing derivation (interlace collapses onto the base rate).
    fn hsyncmode(&self) -> HsyncMode {
        if self.sysport.crt_mode_24khz() {
            HsyncMode::Khz24_8
        } else {
            HsyncMode::Khz15_98
        }
    }

    /// The renderer's horizontal sync mode, including the interlace variant
    /// selected by `grmode` bit 7 (matching `videova_hsyncmode`).
    fn renderer_hsyncmode(&self) -> HsyncModeVa {
        let interlace = self.video.grmode & 0x0080 != 0;
        if self.sysport.crt_mode_24khz() {
            if interlace {
                HsyncModeVa::Khz15_73
            } else {
                HsyncModeVa::Khz24_8
            }
        } else if interlace {
            HsyncModeVa::Khz15_73
        } else {
            HsyncModeVa::Khz15_98
        }
    }

    /// The composed RGBA framebuffer from the last rendered frame.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// The valid `(width, height)` of the composed framebuffer.
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// The VA kanji / font ROM image.
    pub fn font_rom_data(&self) -> &[u8] {
        self.memory.font_rom()
    }

    /// Handles a write to the system-memory-bank port (`0x153`). A change to
    /// the GMSP bit resets the graphics controller; setting it additionally
    /// resets the SGP, matching `memctrlva_o153`.
    pub(crate) fn write_sysm_bank_io(&mut self, value: u8) {
        let changed = self.memory.gmsp_bit() ^ (value & 0x10);
        self.memory.io_write_byte(0x153, value);
        if changed != 0 {
            self.gactrlva.reset();
            if value & 0x10 != 0 {
                self.sgp_reset();
            }
        }
        self.gactrlva.set_single_plane(value & 0x10 != 0);
    }

    /// Resets the Super Graphic Processor, cancelling any pending completion.
    /// The GMSP transition path resets it here.
    fn sgp_reset(&mut self) {
        self.sgp.reset();
        self.scheduler.cancel(Event88Va::SgpComplete);
        self.update_next_event_cycle();
    }

    /// Renders one frame into the renderer's framebuffer at VSYNC.
    /// Applies a sprite-table write produced by a TSP command (CURDEF cursor
    /// enable or a SPRDEF stream byte) into text VRAM.
    fn apply_tsp_mem_effect(&mut self, effect: Option<TspMemEffect>) {
        let text_vram = self.memory.text_vram_mut();
        match effect {
            Some(TspMemEffect::SpriteEnable { offset, enable }) => {
                let index = usize::from(offset);
                if index + 1 < text_vram.len() {
                    let mut word =
                        u16::from(text_vram[index]) | (u16::from(text_vram[index + 1]) << 8);
                    if enable {
                        word |= 0x0200;
                    } else {
                        word &= !0x0200;
                    }
                    text_vram[index] = (word & 0xFF) as u8;
                    text_vram[index + 1] = (word >> 8) as u8;
                }
            }
            Some(TspMemEffect::WriteByte { offset, value }) => {
                let index = usize::from(offset);
                if index < text_vram.len() {
                    text_vram[index] = value;
                }
            }
            None => {}
        }
    }

    fn render_frame(&mut self) {
        let inputs = RenderInputsVa {
            text_vram: self.memory.text_vram(),
            text_table: usize::from(self.tsp.texttable),
            attr_offset: usize::from(self.tsp.attroffset),
            line_height: usize::from(self.tsp.lineheight),
            horizontal_line_position: usize::from(self.tsp.hlinepos),
            blink_counter2: self.tsp.blinkcnt2,
            text_magnify: self.tsp.textmg,
            screen_lines: usize::from(self.tsp.screenlines),
            sync_param0: self.tsp.sync_param0(),
            hsync_mode: self.renderer_hsyncmode(),
            sprite_table: usize::from(self.tsp.sprtable),
            sprite_enabled: self.tsp.spron,
            sprite_count_limit: self.tsp.hspn,
            sprite_magnify: self.tsp.mg,
            sprite_grouping: self.tsp.gr,
            cursor_sprite: self.tsp.curn,
            cursor_blink_enable: self.tsp.be,
            txtmode8: self.video.txtmode8,
            txtmode: self.video.txtmode,
            graphics_mode: self.video.grmode,
            graphics_resolution: self.video.grres,
            color_composition: self.video.colcomp,
            rgb_composition: self.video.rgbcomp,
            palette_mode: self.video.palmode,
            page_mask: self.video.pagemsk,
            backdrop_color: self.video.dropcol,
            transparent_text_sprite: self.video.xpar_txtspr,
            transparent_graphic0: self.video.xpar_g0,
            transparent_graphic1: self.video.xpar_g1,
            mask_mode: self.video.mskmode,
            mask_left: self.video.mskleft,
            mask_right: self.video.mskrit,
            mask_top: self.video.msktop,
            mask_bottom: self.video.mskbot,
            palette_blink_counter: self.video.blinkcnt,
            palette: &self.video.palette,
            graphics_vram: self.memory.graphics_vram(),
            framebuffers: self.video.framebuffer,
        };
        let (width, height) = self.renderer.render(&inputs);
        self.display_width = width;
        self.display_height = height;
    }

    fn trace_presentation(&mut self) {
        if !T::ENABLED {
            return;
        }
        self.presented_frames = self.presented_frames.saturating_add(1);
        self.tracer.trace(
            TraceContext::presentation_main(
                self.current_cycle,
                Some(u64::from(self.clocks.main_clock_hz)),
            ),
            TraceEvent::Presentation(TracePresentation {
                display: trace_id::display::MAIN,
                frame: self.presented_frames,
                width: self.display_width,
                height: self.display_height,
            }),
        );
    }

    /// Advances the TSP frame loop, toggling the display and VSYNC phases.
    fn on_tsp_frame(&mut self) {
        match self.tsp.frame_phase {
            FramePhase::DisplayStart => {
                let mode = self.hsyncmode();
                self.tsp.update_clock(self.clocks.main_clock_hz, mode);
                self.tsp.vsync = 0;
                self.render_frame();
                self.trace_presentation();
                self.tsp.frame_phase = FramePhase::Vsync;
                self.scheduler
                    .schedule(Event88Va::TspFrame, self.current_cycle + self.tsp.dispclock);
                // Re-arm the system-port-4 chain from its End step.
                self.tsp.sysp4_phase = Sysp4Phase::End;
                self.scheduler.schedule(
                    Event88Va::Sysp4Vsync,
                    self.current_cycle + self.tsp.sysp4vsyncextension,
                );
            }
            FramePhase::Vsync => {
                self.tsp.vsync = 0x40;
                self.tsp.tick_blink();
                self.video.tick_blink();
                self.tsp.frame_phase = FramePhase::DisplayStart;
                self.scheduler.schedule(
                    Event88Va::TspFrame,
                    self.current_cycle + self.tsp.vsyncclock,
                );
            }
        }
    }

    /// Advances the system-port-4 VSYNC window and raises the VSYNC IRQ.
    fn on_sysp4_vsync(&mut self) {
        match self.tsp.sysp4_phase {
            Sysp4Phase::End => {
                self.tsp.sysp4vsync = 0;
                self.pic.clear_irq(2);
                self.tsp.sysp4_phase = Sysp4Phase::Start;
                self.scheduler.schedule(
                    Event88Va::Sysp4Vsync,
                    self.current_cycle + self.tsp.sysp4dispclock,
                );
            }
            Sysp4Phase::Start => {
                self.tsp.sysp4vsync = 0x20;
                self.tsp.sysp4_phase = Sysp4Phase::Int;
                self.scheduler
                    .schedule(Event88Va::Sysp4Vsync, self.current_cycle + 6);
            }
            Sysp4Phase::Int => {
                self.pic.set_irq(2);
            }
        }
    }
}

impl<T: TraceSink> Pc88VaBus<T> {
    /// Loads the floppy sub-CPU ROM (8 KiB) into the sub-CPU memory.
    pub(crate) fn load_disk_rom(&mut self, data: &[u8]) {
        self.sub_mem.load_rom(data);
    }

    /// Whether the floppy sub-CPU has a pending FDC interrupt. When the main-CPU
    /// DMA path owns the FDC, its interrupt is routed to the main 8259 instead,
    /// so the sub-CPU must not see (and consume) it.
    pub(crate) fn sub_irq_pending(&self) -> bool {
        if self.fdc_dma_mode {
            return false;
        }
        self.fdc.state.interrupt_pending || self.fdc.pio_byte_ready()
    }

    /// Acknowledges the sub-CPU interrupt, returning the 0x00 ack byte and
    /// leaving the level-sensitive FDC line asserted until the FDC condition is
    /// cleared by software.
    pub(crate) fn acknowledge_sub_irq(&mut self) -> u8 {
        0x00
    }

    /// Advances the sub-CPU T-state position. The sub clock is tracked separately
    /// from the main-unit `current_cycle`, which alone drives scheduled events.
    pub(crate) fn set_sub_cycle(&mut self, cycle: u64) {
        self.sub_cycle = cycle;
    }

    /// Mounts a floppy image into a drive and marks the FDC drive as occupied.
    pub(crate) fn insert_floppy(
        &mut self,
        drive: usize,
        image: device::floppy::FloppyImage,
        path: Option<std::path::PathBuf>,
    ) {
        self.floppy.insert_drive(drive, image, path);
        if drive < 4 {
            self.fdc.state.drive_has_disk |= 1 << drive;
            self.signal_drive_ready_change(drive, true);
        }
    }

    /// Ejects the floppy from a drive and clears the FDC drive-occupied bit.
    pub(crate) fn eject_floppy(&mut self, drive: usize) {
        self.floppy.eject_drive(drive);
        if drive < 4 {
            self.fdc.state.drive_has_disk &= !(1 << drive);
            self.signal_drive_ready_change(drive, false);
        }
    }

    /// Raises the uPD765A ready-line-change interrupt for a runtime media swap
    /// so system software can invalidate its cached directory. The initial
    /// disk mounts happen before the machine runs (cycle 0); those must not
    /// raise an interrupt, since the boot FDC reset has not run yet.
    fn signal_drive_ready_change(&mut self, drive: usize, present: bool) {
        if self.current_cycle == 0 {
            return;
        }
        self.fdc.signal_ready_line_change(drive, present);
        self.update_main_fdc_irq();
    }

    /// Flushes any dirty mounted floppies back to their source files.
    pub(crate) fn flush_floppies(&mut self) {
        self.floppy.flush_all_drives();
    }
}

/// Floppy sub-CPU (PC80S31K) view over the bus: implements `common::Bus` with
/// the disk unit's memory and I/O maps. Built per run slice; never coexists with
/// the main-CPU access.
pub(crate) struct SubBusView<'a, T: TraceSink> {
    pub(crate) bus: &'a mut Pc88VaBus<T>,
}

impl<T: TraceSink> Bus for SubBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let value = self.bus.sub_mem.read(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Read,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address as u16;
        self.bus.sub_mem.write(address, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let value = self.bus.sub_mem.read(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Fetch,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        let (value, handled) = self.bus.sub_io_read(port);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_IO,
                    TraceAccessKind::Read,
                    u64::from(port & 0xFF),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        let handled = self.bus.sub_io_write(port, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_IO,
                    TraceAccessKind::Write,
                    u64::from(port & 0xFF),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
    }

    fn has_irq(&self) -> bool {
        self.bus.sub_irq_pending()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let vector = self.bus.acknowledge_sub_irq();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.clocks.sub_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::PC88VA_SUB_FDC,
                    0,
                    TraceInterruptAction::Acknowledge,
                    Some(u32::from(vector)),
                ),
            );
        }
        vector
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    // The sub CPU runs in its own clock domain, not the shared
    // main-unit `current_cycle` that drives the scheduler.
    #[allow(clippy::misnamed_getters)]
    fn current_cycle(&self) -> u64 {
        self.bus.sub_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.bus.set_sub_cycle(cycle);
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.bus.tracer.yield_requested()
    }
}

impl<T: TraceSink> Bus for Pc88VaBus<T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let value = if let Some(offset) = self.memory.graphics_window_offset(address) {
            self.gactrlva
                .gvram_read(self.memory.graphics_vram(), offset)
        } else {
            self.memory.read_byte(address)
        };
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Read,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        if let Some(offset) = self.memory.graphics_window_offset(address) {
            self.gactrlva
                .gvram_write(self.memory.graphics_vram_mut(), offset, value);
        } else {
            self.memory.write_byte(address, value);
        }
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let value = if let Some(offset) = self.memory.graphics_window_offset(address) {
            self.gactrlva
                .gvram_read(self.memory.graphics_vram(), offset)
        } else {
            self.memory.read_byte(address)
        };
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Fetch,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        let (value, handled) = self.io_read(port);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_IO,
                    TraceAccessKind::Read,
                    u64::from(port),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        let handled = self.io_write(port, value);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_IO,
                    TraceAccessKind::Write,
                    u64::from(port),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
    }

    fn has_irq(&self) -> bool {
        self.pic.has_pending_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let acknowledge = self.pic.acknowledge_with_line();
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.main_clock_hz)),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::PC88VA_PIC,
                    TraceInterruptKind::Maskable,
                    acknowledge.line.map(u16::from),
                    TraceInterruptAction::Acknowledge,
                    Some(u32::from(acknowledge.vector)),
                ),
            );
        }
        acknowledge.vector
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
        if cycle >= self.next_event_cycle {
            self.process_events();
        }
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.tracer.yield_requested()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use common::{
        Bus, TraceAccessKind, TraceContext, TraceEvent, TraceInterrupt, TraceSink, trace_id,
        trace_source,
    };

    use super::{Pc88VaBus, SubBusView};
    use crate::{
        config::{ClockConfig, Pc88VaModel},
        memory::Pc88VaMemory,
        rom::LoadedRoms,
    };

    /// Minimal correctly-sized ROM set for device-level bus tests. The floppy
    /// and PPI paths do not need the real BIOS images, only valid sizes.
    fn stub_roms() -> LoadedRoms {
        LoadedRoms {
            rom00: vec![0u8; 0x8_0000],
            rom08: vec![0u8; 0x2_0000],
            rom1: vec![0u8; 0x2_0000],
            font: vec![0u8; 0x5_0000],
            dictionary: vec![0u8; 0x8_0000],
            subsys: vec![0u8; 0x2000],
        }
    }

    /// Builds a `Pc88VaBus` with stub ROMs for in-crate unit tests.
    pub(crate) fn test_bus() -> Pc88VaBus {
        let model = Pc88VaModel::PC88VA2;
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sub_clock_hz: model.sub_clock_hz(),
            sample_rate: 48_000,
        };
        let memory = Pc88VaMemory::new(model, stub_roms());
        Pc88VaBus::from_parts(memory, clocks)
    }

    #[derive(Default)]
    struct InterruptTrace {
        events: Vec<(TraceContext, TraceInterrupt)>,
        accesses: Vec<common::TraceAccess>,
    }

    impl TraceSink for InterruptTrace {
        fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Interrupt(interrupt) => self.events.push((context, interrupt)),
                TraceEvent::Access(access) => self.accesses.push(access),
                _ => {}
            }
        }
    }

    #[test]
    fn pc80s31k_mailbox_crosses_data_ports() {
        let mut bus = test_bus();

        bus.io_write(0xFC, 0x5A);
        assert_eq!(bus.sub_io_read(0xFD).0, 0x5A);

        bus.sub_io_write(0xFC, 0xA5);
        assert_eq!(bus.io_read(0xFD).0, 0xA5);
    }

    #[test]
    fn pc80s31k_port_c_strobes_cross_nibbles_and_arm_resync() {
        let mut bus = test_bus();

        bus.current_cycle = 100;
        bus.io_write(0xFF, (4 << 1) | 1);
        assert_eq!(bus.sub_io_read(0xFE).0 & 0x0F, 0x01);
        assert_eq!(bus.resync_until, bus.current_cycle + super::SYNC_SLICE);

        bus.current_cycle = 200;
        bus.sub_io_write(0xFF, 0x01);
        assert_eq!(bus.io_read(0xFE).0 & 0xF0, 0x10);
        assert_eq!(bus.resync_until, bus.current_cycle + super::SYNC_SLICE);
    }

    #[test]
    fn main_and_sub_interrupt_acknowledgements_are_traced() {
        let model = Pc88VaModel::PC88VA2;
        let mut bus =
            Pc88VaBus::new_with_trace_sink(model, stub_roms(), 48_000, InterruptTrace::default());
        Bus::fetch_opcode_byte(&mut bus, 0);
        Bus::io_read_byte(&mut bus, 0xFFFF);
        {
            let mut sub = SubBusView { bus: &mut bus };
            Bus::io_read_byte(&mut sub, 0x01);
        }
        bus.pic.write_port2(0, 0);
        bus.pic.write_port2(1, 0);
        for line in 0..16 {
            bus.pic.clear_irq(line);
        }
        bus.pic.set_irq(12);
        Bus::acknowledge_irq(&mut bus);
        {
            let mut sub = SubBusView { bus: &mut bus };
            Bus::acknowledge_irq(&mut sub);
        }

        assert_eq!(bus.tracer().events.len(), 2);
        let (main_context, main_interrupt) = bus.tracer().events[0];
        assert_eq!(main_context.source, trace_source::CPU_MAIN);
        assert_eq!(main_interrupt.controller, trace_id::controller::PC88VA_PIC);
        assert_eq!(main_interrupt.line, Some(12));
        let (sub_context, sub_interrupt) = bus.tracer().events[1];
        assert_eq!(sub_context.source, trace_source::CPU_SUB);
        assert_eq!(
            sub_interrupt.controller,
            trace_id::controller::PC88VA_SUB_FDC
        );
        assert_eq!(sub_interrupt.line, Some(0));
        assert_eq!(sub_interrupt.vector, Some(0));
        assert_eq!(bus.tracer().accesses[0].kind, TraceAccessKind::Fetch);
        assert!(!bus.tracer().accesses[1].handled);
        assert!(!bus.tracer().accesses[2].handled);
    }
}
