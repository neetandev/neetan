//! PC/AT system bus: owns memory and chipset devices, dispatches I/O, and
//! drives the event scheduler.

mod atapi;
mod fdc;
mod hdd;
mod io_read;
mod io_write;
mod keyboard;
mod serial;
mod sound;

use common::{
    Bus, HostDateTime, HostDateTimeProvider, NoTrace, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceContext, TraceEvent, TraceInterruptAction, TraceInterruptKind,
    TracePresentation, TraceSink, default_host_date_time, trace_id,
};
use device::{
    at_dma::AtDma,
    beeper::Beeper,
    cs4031::Cs4031,
    gameport::GamePort,
    i8042_kbc::I8042Kbc,
    i8253_pit::I8253Pit,
    i8259a_pic::{I8259aPic, I8259aPicState},
    ide::{AtAtapiController, AtIdeController},
    ins8250_uart::Ins8250Uart,
    mc146818_rtc::Mc146818Rtc,
    mouse_serial::SerialMouse,
    mpu401::Mpu401,
    sound_blaster_16::{SB16_PLATFORM_ISA_AT, SoundBlaster16},
    upd765a_fdc::{UPD765_PLATFORM_ISA_AT, Upd765aFdc},
    vga::{RetraceStatus, Vga, VgaRenderMode as DeviceVgaRenderMode},
};
pub use keyboard::{
    AT_KEY_CURSOR_DOWN, AT_KEY_CURSOR_LEFT, AT_KEY_CURSOR_RIGHT, AT_KEY_CURSOR_UP, AT_KEY_DELETE,
    AT_KEY_END, AT_KEY_HOME, AT_KEY_INSERT, AT_KEY_KEYPAD_DIVIDE, AT_KEY_KEYPAD_ENTER,
    AT_KEY_PAGE_DOWN, AT_KEY_PAGE_UP, AT_KEY_RIGHT_ALT, AT_KEY_RIGHT_CTRL,
};
use software_renderer::{
    RenderInputsVga, VGA_FALLBACK_HEIGHT, VGA_FALLBACK_WIDTH, VgaRenderMode, VgaRenderer,
};

use crate::{
    cmos::initial_cmos,
    config::{ClockConfig, PIT_CLOCK_HZ},
    memory::{AtMemory, UMA_BASE, VGA_WINDOW_BASE},
    rom::LoadedRoms,
    scheduler::{AtScheduler, EventAt},
};

/// IRQ line: PIT channel 0 timer tick.
const IRQ_TIMER: u8 = 0;
/// IRQ line: keyboard.
const IRQ_KEYBOARD: u8 = 1;
/// IRQ line: COM1 serial port.
const IRQ_COM1: u8 = 4;
/// IRQ line: floppy disk controller.
const IRQ_FDC: u8 = 6;
/// IRQ line: RTC periodic/alarm/update.
const IRQ_RTC: u8 = 8;
/// IRQ line: MPU-401 MIDI interface.
const IRQ_MPU: u8 = 9;
/// IRQ line: FPU error (FERR#).
const IRQ_FPU: u8 = 13;
/// IRQ line: IDE primary channel.
const IRQ_IDE_PRIMARY: u8 = 14;
/// IRQ line: IDE secondary channel (ATAPI CD-ROM).
const IRQ_IDE_SECONDARY: u8 = 15;

/// 8042 output-buffer delivery interval in microseconds.
const KBC_DELIVER_MICROS: u64 = 250;

/// Value returned by an open-bus read.
pub(crate) const OPEN_BUS_BYTE: u8 = 0xFF;

/// Number of 64-bit words needed to hold one bit per 16-bit I/O port.
const PORT_BITMAP_WORDS: usize = 1024;

/// PC/AT system bus.
pub struct AtBus<T: TraceSink = NoTrace> {
    /// Physical memory and the shadow/A20 decode.
    pub(crate) memory: AtMemory,
    /// Clock configuration.
    pub(crate) clocks: ClockConfig,
    /// Monotonic CPU cycle counter.
    pub(crate) current_cycle: u64,
    /// Cached next scheduled event cycle.
    pub(crate) next_event_cycle: u64,
    /// Event scheduler.
    pub(crate) scheduler: AtScheduler,
    /// CS4031 chipset configuration core.
    pub(crate) chipset: Cs4031,
    /// Cascaded 8259A interrupt controller pair.
    pub(crate) pic: I8259aPic,
    /// 8254 programmable interval timer.
    pub(crate) pit: I8253Pit,
    /// Dual-8237 DMA front-end.
    pub(crate) dma: AtDma,
    /// AT floppy disk controller with its two drives.
    pub(crate) fdc: Upd765aFdc<UPD765_PLATFORM_ISA_AT>,
    /// Whether the next FDC interrupt event delivers the reset polling drain.
    pub(crate) fdc_reset_poll_pending: bool,
    /// IDE primary channel with up to two hard drives.
    pub(crate) ide: AtIdeController,
    /// IDE secondary channel with the ATAPI CD-ROM drive.
    pub(crate) ide_secondary: AtAtapiController,
    /// MC146818 RTC and CMOS RAM.
    pub(crate) rtc: Mc146818Rtc,
    /// 8042 keyboard controller and AT keyboard.
    pub(crate) kbc: I8042Kbc,
    /// ET4000AX display adapter.
    pub(crate) vga: Vga,
    /// VGA software renderer.
    pub(crate) renderer: VgaRenderer,
    /// Frame width of the most recent rendered frame.
    pub(crate) display_width: u32,
    /// Frame height of the most recent rendered frame.
    pub(crate) display_height: u32,
    /// Number assigned to the next published frame.
    pub(crate) presented_frames: u64,
    /// Cycle of the most recent VGA frame event (vertical sync start).
    pub(crate) last_vsync_start_cycle: u64,
    /// PC speaker beeper.
    pub(crate) beeper: Beeper,
    /// Sound Blaster 16 (CT1741 DSP, CT1745 mixer, YMF262 OPL3).
    pub(crate) sound_blaster_16: SoundBlaster16<SB16_PLATFORM_ISA_AT>,
    /// MPU-401 MIDI interface.
    pub(crate) mpu401: Mpu401,
    /// COM1 16450 UART (serial mouse).
    pub(crate) uart_com1: Ins8250Uart,
    /// Microsoft serial mouse attached to COM1.
    pub(crate) serial_mouse: SerialMouse,
    /// Standard analog game port (0x200-0x207).
    pub(crate) gameport: GamePort,
    /// Roland MT-32 module driven from the MPU-401 output.
    #[cfg(feature = "mt32")]
    pub(crate) mt32: Option<device::mt32::Mt32>,
    /// Roland SC-55 module driven from the MPU-401 output.
    #[cfg(feature = "sc55")]
    pub(crate) sc55: Option<device::sc55::Sc55>,
    /// Port 0x61 timer-2 gate latch (rising-edge detection for channel 2).
    pub(crate) timer2_gate: bool,
    /// FPU busy latch (FERR#); cleared by a port 0xF0/0xF1 write.
    pub(crate) fpu_busy_latch: bool,
    /// Pending CPU reset requested by the KBC or CS4031.
    pub(crate) cpu_reset_pending: bool,
    /// Pending wait-state cycles.
    pub(crate) pending_wait_cycles: i64,
    /// Most recent POST diagnostic code (port 0x80).
    pub(crate) last_post_code: u8,
    /// Log-once bitmap for unhandled read ports.
    unhandled_read_logged: Box<[u64; PORT_BITMAP_WORDS]>,
    /// Log-once bitmap for unhandled write ports.
    unhandled_write_logged: Box<[u64; PORT_BITMAP_WORDS]>,
    /// Host date-time provider used to seed the RTC.
    host_date_time_provider: HostDateTimeProvider,
    /// TraceSink sink.
    pub(crate) tracer: T,
}

impl<T: TraceSink> AtBus<T> {
    /// Builds a traced bus with the requested RAM, ROMs, and sample rate.
    pub fn new_with_trace_sink(
        cpu_clock_hz: u32,
        ram_size: u32,
        roms: LoadedRoms,
        sample_rate: u32,
        tracer: T,
    ) -> Self {
        let provider: HostDateTimeProvider = default_host_date_time;
        let cmos_seed = initial_cmos(ram_size as usize);
        let seed = provider();

        let mut bus = Self {
            memory: AtMemory::new(ram_size, roms.system_bios, roms.vga_bios),
            clocks: ClockConfig::new(cpu_clock_hz, sample_rate),
            current_cycle: 0,
            next_event_cycle: u64::MAX,
            scheduler: AtScheduler::new(),
            chipset: Cs4031::new(),
            pic: I8259aPic::new_zeroed(),
            pit: I8253Pit::new_zeroed(),
            dma: AtDma::new(),
            fdc: Upd765aFdc::new(),
            fdc_reset_poll_pending: false,
            ide: AtIdeController::new(),
            ide_secondary: AtAtapiController::new(sample_rate),
            rtc: Mc146818Rtc::new(seed, &cmos_seed),
            kbc: I8042Kbc::new(),
            vga: Vga::new(),
            renderer: VgaRenderer::new(),
            display_width: VGA_FALLBACK_WIDTH,
            display_height: VGA_FALLBACK_HEIGHT,
            presented_frames: 0,
            last_vsync_start_cycle: 0,
            beeper: Beeper::new(common::BeeperKind::PitDriven, PIT_CLOCK_HZ),
            sound_blaster_16: SoundBlaster16::new(cpu_clock_hz, sample_rate),
            mpu401: Mpu401::new(),
            uart_com1: Ins8250Uart::new(cpu_clock_hz),
            serial_mouse: SerialMouse::new(),
            gameport: GamePort::new(cpu_clock_hz),
            #[cfg(feature = "mt32")]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: None,
            timer2_gate: false,
            fpu_busy_latch: false,
            cpu_reset_pending: false,
            pending_wait_cycles: 0,
            last_post_code: 0,
            unhandled_read_logged: Box::new([0; PORT_BITMAP_WORDS]),
            unhandled_write_logged: Box::new([0; PORT_BITMAP_WORDS]),
            host_date_time_provider: provider,
            tracer,
        };
        bus.memory.refresh_uma(&bus.chipset);
        bus.reschedule_rtc_update();
        bus.schedule_next_vga_frame();
        bus
    }
}

impl AtBus<NoTrace> {
    /// Builds an untraced bus with the requested RAM, ROMs, and sample rate.
    pub fn new(cpu_clock_hz: u32, ram_size: u32, roms: LoadedRoms, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(cpu_clock_hz, ram_size, roms, sample_rate, NoTrace)
    }
}

impl<T: TraceSink> AtBus<T> {
    /// Installs the host date-time provider and reseeds the RTC from it.
    pub fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
        let now: HostDateTime = provider();
        self.rtc.reseed_time(now);
    }

    /// A shared reference to the bus-activity tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// A mutable reference to the bus-activity tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Returns the current PIC state.
    pub fn pic_state(&self) -> &I8259aPicState {
        &self.pic.state
    }

    /// Raises an IRQ line and notifies the tracer.
    pub(crate) fn raise_irq(&mut self, irq: u8) {
        let changed = self.pic.set_irq(irq);
        if T::ENABLED && changed {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::AT_PIC,
                    u16::from(irq),
                    TraceInterruptAction::Assert,
                    None,
                ),
            );
        }
    }

    /// Clears an IRQ line and notifies the tracer.
    pub(crate) fn clear_irq(&mut self, irq: u8) {
        let changed = self.pic.clear_irq(irq);
        if T::ENABLED && changed {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::AT_PIC,
                    u16::from(irq),
                    TraceInterruptAction::Clear,
                    None,
                ),
            );
        }
    }

    /// Sets the game-port buttons at `index` from the digital joystick state.
    pub fn set_joystick(&mut self, index: usize, state: common::JoystickState) {
        self.gameport
            .set_buttons(index, state.trigger1, state.trigger2);
    }

    /// Sets the analog game-port axes at `index` and marks the stick present.
    ///
    /// The host forwards this only while a real gamepad is connected, so it
    /// doubles as the presence signal for the port.
    pub fn set_joystick_axes(&mut self, index: usize, x: i16, y: i16) {
        self.gameport.set_present(index, true);
        self.gameport.set_axes(index, x, y);
    }

    /// Returns the configured CPU clock in hertz.
    pub fn cpu_clock_hz(&self) -> u32 {
        self.clocks.cpu_clock_hz
    }

    /// Returns the derived clock and wait-state configuration.
    pub fn clock_config(&self) -> ClockConfig {
        self.clocks
    }

    /// Returns the rendered display framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// A shared reference to the VGA device (register dumps and tests).
    pub fn vga(&self) -> &Vga {
        &self.vga
    }

    /// Returns the dimensions of the most recent rendered frame.
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// Mixes every sound source into the output buffer for one audio frame.
    ///
    /// The buffer arrives pre-zeroed from the audio engine, so each source mixes
    /// additively: the PC speaker, the Sound Blaster 16 (OPL3 FM plus DSP PCM),
    /// and any MIDI module driven from the MPU-401.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let frame_end = self.current_cycle;
        let cpu_clock_hz = self.clocks.cpu_clock_hz;

        self.beeper.mix_samples(
            frame_end,
            cpu_clock_hz,
            PIT_CLOCK_HZ,
            self.clocks.sample_rate,
            volume,
            output,
        );

        self.sound_blaster_16
            .generate_samples(frame_end, cpu_clock_hz, volume, output);

        self.ide_secondary.generate_cd_audio_samples(volume, output);

        #[cfg(feature = "mt32")]
        if let Some(ref mt32) = self.mt32 {
            mt32.exchange(volume, output, |buf| self.mpu401.flush_midi_into(buf));
        }
        #[cfg(feature = "sc55")]
        if let Some(ref sc55) = self.sc55 {
            sc55.exchange(volume, output, |buf| self.mpu401.flush_midi_into(buf));
        }

        output.len()
    }

    /// Returns the current CD audio playback state and positions.
    pub fn cd_audio_status(&self) -> Option<common::CdAudioStatus> {
        if !self.ide_secondary.has_cdrom() {
            return None;
        }

        let player = self.ide_secondary.cd_audio_player();
        let (current_lba, start_lba, end_lba) = player.current_position();
        let state = match player.state() {
            device::cd_audio::CdAudioState::Stopped => common::CdAudioState::Stopped,
            device::cd_audio::CdAudioState::Playing => common::CdAudioState::Playing,
            device::cd_audio::CdAudioState::Paused => common::CdAudioState::Paused,
        };
        Some(common::CdAudioStatus {
            state,
            current_lba,
            start_lba,
            end_lba,
        })
    }

    /// Installs a Roland MT-32 module driven from the MPU-401 output.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.mt32 = Some(device::mt32::Mt32::new(rom_directory)?);
        Ok(())
    }

    /// Installs a Roland SC-55 module driven from the MPU-401 output.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.sc55 = Some(device::sc55::Sc55::new(rom_directory)?);
        Ok(())
    }

    /// Returns whether a CPU reset is pending, consuming the flag.
    pub fn take_cpu_reset(&mut self) -> bool {
        core::mem::take(&mut self.cpu_reset_pending)
    }

    /// Applies a CS4031 effect set to the bus.
    pub(crate) fn apply_chipset_effects(&mut self, effects: device::cs4031::Cs4031Effects) {
        if effects.shadow_map_changed {
            self.memory.refresh_uma(&self.chipset);
        }
        if effects.a20_changed {
            self.memory.set_a20(self.chipset.a20_enabled());
        }
        if effects.cpu_reset_pulse {
            self.cpu_reset_pending = true;
        }
    }

    /// Applies an 8042 effect set to the bus.
    pub(crate) fn apply_kbc_effects(&mut self, effects: device::i8042_kbc::KbcEffects) {
        if effects.output_port_changed {
            let chipset_effects = self.chipset.set_ext_gate_a20(self.kbc.a20_enabled());
            self.apply_chipset_effects(chipset_effects);
        }
        if effects.reset_pulse {
            let chipset_effects = self.chipset.kbc_reset_line(false);
            self.apply_chipset_effects(chipset_effects);
            // The line returns high after the pulse.
            let _ = self.chipset.kbc_reset_line(true);
        }
        if effects.schedule_delivery {
            self.schedule_kbc_deliver();
        }
    }

    /// Recomputes the cached next-event cycle.
    fn update_next_event_cycle(&mut self) {
        self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
    }

    /// Returns the earliest scheduled event cycle, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// (Re)schedules the PIT channel 0 timer event from its current period.
    pub(crate) fn reschedule_pit_channel0(&mut self) {
        self.scheduler.cancel(EventAt::PitChannel0Low);
        let period = self
            .pit
            .timer0_period_cycles(self.clocks.cpu_clock_hz, PIT_CLOCK_HZ)
            .max(1);
        self.scheduler.schedule(
            EventAt::PitChannel0,
            self.current_cycle.saturating_add(period),
        );
        self.update_next_event_cycle();
    }

    /// Schedules the next one-second RTC update and anchors the UIP window.
    pub(crate) fn reschedule_rtc_update(&mut self) {
        let period = u64::from(self.clocks.cpu_clock_hz).max(1);
        let fire = self.current_cycle.saturating_add(period);
        self.scheduler.schedule(EventAt::RtcUpdate, fire);
        self.rtc.set_next_update_cycle(fire);
        self.update_next_event_cycle();
    }

    /// (Re)schedules or cancels the RTC periodic event from its rate.
    pub(crate) fn reschedule_rtc_periodic(&mut self) {
        match self.rtc.periodic_period_cycles(self.clocks.cpu_clock_hz) {
            Some(period) => self.scheduler.schedule(
                EventAt::RtcPeriodic,
                self.current_cycle.saturating_add(period.max(1)),
            ),
            None => self.scheduler.cancel(EventAt::RtcPeriodic),
        }
        self.update_next_event_cycle();
    }

    /// Schedules the next VGA frame event one frame period ahead.
    pub(crate) fn schedule_next_vga_frame(&mut self) {
        let period = self
            .vga
            .frame_timing()
            .frame_cycles(self.clocks.cpu_clock_hz);
        self.scheduler
            .schedule(EventAt::VgaFrame, self.current_cycle.saturating_add(period));
        self.update_next_event_cycle();
    }

    /// Computes the live retrace state for input status one reads.
    pub(crate) fn vga_retrace_status(&self) -> RetraceStatus {
        let cycles_into_frame = self.current_cycle - self.last_vsync_start_cycle;
        self.vga
            .frame_timing()
            .retrace_status(cycles_into_frame, self.clocks.cpu_clock_hz)
    }

    /// Resolves the VGA state and renders one frame.
    pub(crate) fn render_frame(&mut self) {
        let resolved = self.vga.resolve();
        let render_mode = match resolved.render_mode {
            DeviceVgaRenderMode::Text => VgaRenderMode::Text,
            DeviceVgaRenderMode::Planar16 => VgaRenderMode::Planar16,
            DeviceVgaRenderMode::Packed256 => VgaRenderMode::Packed256,
            DeviceVgaRenderMode::CgaInterleaved => VgaRenderMode::CgaInterleaved,
            DeviceVgaRenderMode::Mono1bpp => VgaRenderMode::Mono1bpp,
        };
        let inputs = RenderInputsVga {
            vram: self.vga.vram(),
            render_mode,
            blanked: resolved.blanked,
            columns: resolved.columns,
            character_width: resolved.character_width,
            character_height: resolved.character_height,
            scan_doubled: resolved.scan_doubled,
            active_scanlines: resolved.active_scanlines,
            start_address: resolved.start_address,
            row_pitch: resolved.row_pitch,
            address_step: resolved.address_step,
            plane_address_mask: resolved.plane_address_mask,
            map13_from_row_scan: resolved.map13_from_row_scan,
            map14_from_row_scan: resolved.map14_from_row_scan,
            line_compare: resolved.line_compare,
            pel_pan_reset_on_split: resolved.pel_pan_reset_on_split,
            preset_row_scan: resolved.preset_row_scan,
            cursor_address: resolved.cursor_address,
            cursor_start_row: resolved.cursor_start_row,
            cursor_end_row: resolved.cursor_end_row,
            cursor_visible: resolved.cursor_visible,
            blink_enabled: resolved.blink_enabled,
            blink_visible: resolved.blink_visible,
            line_graphics: resolved.line_graphics,
            font_offset_map_a: resolved.font_offset_map_a,
            font_offset_map_b: resolved.font_offset_map_b,
            pel_pan: resolved.pel_pan,
            packed_half_rate: resolved.packed_half_rate,
            border_color: resolved.border_color,
            pens: resolved.pens,
            pens_256: resolved.pens_256,
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
                Some(u64::from(self.clocks.cpu_clock_hz)),
            ),
            TraceEvent::Presentation(TracePresentation {
                display: trace_id::display::MAIN,
                frame: self.presented_frames,
                width: self.display_width,
                height: self.display_height,
            }),
        );
    }

    /// Schedules the next 8042 output-buffer delivery if output is pending.
    pub(crate) fn schedule_kbc_deliver(&mut self) {
        if self.kbc.has_pending_output() {
            let period =
                (u64::from(self.clocks.cpu_clock_hz) * KBC_DELIVER_MICROS / 1_000_000).max(1);
            self.scheduler.schedule(
                EventAt::KbcDeliver,
                self.current_cycle.saturating_add(period),
            );
            self.update_next_event_cycle();
        }
    }

    /// Dispatches all events due at the current cycle and re-arms periodic ones.
    fn process_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        for event in due.iter() {
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::scheduler_main(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::Scheduled {
                        event: event.kind.trace_name(),
                        fire_tick: event.fire_cycle,
                    },
                );
            }
            match event.kind {
                EventAt::PitChannel0 => {
                    let raise_irq = self.pit.advance_timer0(self.current_cycle);
                    self.reschedule_pit_channel0();
                    if raise_irq {
                        self.pit.channels[0].output = true;
                        self.raise_irq(IRQ_TIMER);
                        if let Some(delay) = self
                            .pit
                            .timer0_high_cycles(self.clocks.cpu_clock_hz, PIT_CLOCK_HZ)
                        {
                            self.scheduler.schedule(
                                EventAt::PitChannel0Low,
                                self.current_cycle.saturating_add(delay),
                            );
                            self.update_next_event_cycle();
                        }
                    }
                }
                EventAt::PitChannel0Low => {
                    self.pit.channels[0].output = false;
                    self.clear_irq(IRQ_TIMER);
                }
                EventAt::RtcUpdate => {
                    if self.rtc.advance_one_second() {
                        self.raise_irq(IRQ_RTC);
                    }
                    self.reschedule_rtc_update();
                }
                EventAt::RtcPeriodic => {
                    if self.rtc.periodic_tick() {
                        self.raise_irq(IRQ_RTC);
                    }
                    self.reschedule_rtc_periodic();
                }
                EventAt::KbcDeliver => {
                    if let Some(true) = self.kbc.deliver_next() {
                        self.raise_irq(IRQ_KEYBOARD);
                    }
                    self.schedule_kbc_deliver();
                }
                EventAt::KeyboardTypematic => {}
                EventAt::VgaFrame => {
                    self.last_vsync_start_cycle = self.current_cycle;
                    self.vga.on_vsync_start();
                    self.render_frame();
                    self.trace_presentation();
                    self.schedule_next_vga_frame();
                }
                EventAt::FdcExecution => self.handle_fdc_execution(),
                EventAt::FdcInterrupt => self.handle_fdc_interrupt(),
                EventAt::IdeExecution => self.handle_ide_execution(),
                EventAt::IdeInterrupt => self.handle_ide_interrupt(),
                EventAt::IdeSecondaryExecution => self.handle_ide_secondary_execution(),
                EventAt::IdeSecondaryInterrupt => self.handle_ide_secondary_interrupt(),
                EventAt::Sb16OplTimerA => self.handle_sb16_opl_timer(
                    device::sound_blaster_16::SoundboardSb16Timer::OplTimerA,
                ),
                EventAt::Sb16OplTimerB => self.handle_sb16_opl_timer(
                    device::sound_blaster_16::SoundboardSb16Timer::OplTimerB,
                ),
                EventAt::Sb16DspDma => self.handle_sb16_dma_transfer(event.fire_cycle),
                EventAt::MpuTimer => self.handle_mpu_timer(),
                EventAt::UartRx => {
                    self.uart_com1.advance_to(self.current_cycle);
                    self.sync_com1_irq();
                    self.reschedule_uart_rx();
                }
            }
        }
        self.update_next_event_cycle();
    }

    /// Computes the port-0x61 refresh-detect bit from PIT channel 1.
    pub(crate) fn refresh_toggle(&self) -> bool {
        let channel = &self.pit.channels[1];
        let reload = if channel.value == 0 {
            0x1_0000u64
        } else {
            u64::from(channel.value)
        };
        let elapsed_cpu = self.current_cycle.saturating_sub(channel.last_load_cycle);
        let elapsed_pit =
            elapsed_cpu * u64::from(PIT_CLOCK_HZ) / u64::from(self.clocks.cpu_clock_hz).max(1);
        (elapsed_pit / reload) & 1 == 1
    }

    /// Computes the port-0x61 timer-2 output bit.
    pub(crate) fn timer2_output(&self) -> bool {
        if self.timer2_gate {
            self.pit.get_output(
                2,
                self.current_cycle,
                self.clocks.cpu_clock_hz,
                PIT_CLOCK_HZ,
            )
        } else {
            true
        }
    }

    /// Records a POST diagnostic code (port 0x80).
    pub(crate) fn record_post_code(&mut self, code: u8) {
        self.last_post_code = code;
        common::debug!("POST {code:#04X}");
    }

    /// Returns the most recent POST diagnostic code.
    pub fn last_post_code(&self) -> u8 {
        self.last_post_code
    }

    /// Returns one CMOS RAM byte (used by tests and diagnostics).
    pub fn cmos_byte(&self, index: usize) -> u8 {
        self.rtc.cmos[index & 0x7F]
    }

    /// Logs an unhandled read port once.
    pub(crate) fn log_unhandled_read(&mut self, port: u16) {
        let word = (port >> 6) as usize;
        let bit = 1u64 << (port & 0x3F);
        if self.unhandled_read_logged[word] & bit == 0 {
            self.unhandled_read_logged[word] |= bit;
            common::warn!("machine_at: unhandled I/O read from port {port:#06X}");
        }
    }

    /// Logs an unhandled write port once.
    pub(crate) fn log_unhandled_write(&mut self, port: u16, value: u8) {
        let word = (port >> 6) as usize;
        let bit = 1u64 << (port & 0x3F);
        if self.unhandled_write_logged[word] & bit == 0 {
            self.unhandled_write_logged[word] |= bit;
            common::warn!("machine_at: unhandled I/O write to port {port:#06X} value {value:#04X}");
        }
    }

    fn read_byte_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u8 {
        let physical = self.memory.apply_a20(address);
        let value = if (VGA_WINDOW_BASE..UMA_BASE).contains(&physical) {
            if self.memory.ab_internal(physical) {
                self.memory.read_physical(physical)
            } else {
                self.pending_wait_cycles += self.clocks.vga_memory_wait_cycles;
                self.vga
                    .mem_read(physical - VGA_WINDOW_BASE)
                    .unwrap_or(OPEN_BUS_BYTE)
            }
        } else {
            self.memory.read_physical(physical)
        };
        if T::ENABLED {
            let kind = if FETCH {
                TraceAccessKind::Fetch
            } else {
                TraceAccessKind::Read
            };
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    kind,
                    u64::from(physical),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    /// Reads one word while preserving data-read or instruction-fetch identity.
    fn read_word_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u16 {
        let physical = self.memory.apply_a20(address);
        if let Some(value) = self.memory.read_ram_word(physical) {
            if T::ENABLED {
                let kind = if FETCH {
                    TraceAccessKind::Fetch
                } else {
                    TraceAccessKind::Read
                };
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_MEMORY,
                        kind,
                        u64::from(physical),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return value;
        }
        let low = self.read_byte_for_cpu::<FETCH>(address) as u16;
        let high = self.read_byte_for_cpu::<FETCH>(address.wrapping_add(1)) as u16;
        low | (high << 8)
    }

    /// Reads one doubleword while preserving data-read or instruction-fetch identity.
    fn read_dword_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u32 {
        let physical = self.memory.apply_a20(address);
        if let Some(value) = self.memory.read_ram_dword(physical) {
            if T::ENABLED {
                let kind = if FETCH {
                    TraceAccessKind::Fetch
                } else {
                    TraceAccessKind::Read
                };
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_MEMORY,
                        kind,
                        u64::from(physical),
                        TraceAccessWidth::Dword,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return value;
        }
        let low = self.read_word_for_cpu::<FETCH>(address) as u32;
        let high = self.read_word_for_cpu::<FETCH>(address.wrapping_add(2)) as u32;
        low | (high << 16)
    }
}

impl<T: TraceSink> Bus for AtBus<T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read_byte_for_cpu::<false>(address)
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        self.read_byte_for_cpu::<true>(address)
    }

    fn fetch_opcode_word(&mut self, address: u32) -> u16 {
        self.read_word_for_cpu::<true>(address)
    }

    fn fetch_opcode_dword(&mut self, address: u32) -> u32 {
        self.read_dword_for_cpu::<true>(address)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let physical = self.memory.apply_a20(address);
        if (VGA_WINDOW_BASE..UMA_BASE).contains(&physical) {
            if self.memory.ab_internal(physical) {
                self.memory.write_physical(physical, value);
            } else {
                self.pending_wait_cycles += self.clocks.vga_memory_wait_cycles;
                self.vga.mem_write(physical - VGA_WINDOW_BASE, value);
            }
        } else {
            self.memory.write_physical(physical, value);
        }
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(physical),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn read_word(&mut self, address: u32) -> u16 {
        self.read_word_for_cpu::<false>(address)
    }

    fn read_dword(&mut self, address: u32) -> u32 {
        self.read_dword_for_cpu::<false>(address)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        let physical = self.memory.apply_a20(address);
        if self.memory.write_ram_word(physical, value) {
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_MEMORY,
                        TraceAccessKind::Write,
                        u64::from(physical),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return;
        }
        self.write_byte(address, value as u8);
        self.write_byte(address.wrapping_add(1), (value >> 8) as u8);
    }

    fn write_dword(&mut self, address: u32, value: u32) {
        let physical = self.memory.apply_a20(address);
        if self.memory.write_ram_dword(physical, value) {
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_MEMORY,
                        TraceAccessKind::Write,
                        u64::from(physical),
                        TraceAccessWidth::Dword,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return;
        }
        self.write_word(address, value as u16);
        self.write_word(address.wrapping_add(2), (value >> 16) as u16);
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        self.pending_wait_cycles += self.clocks.io_8bit_wait_cycles;
        let (value, handled) = self.io_read(port);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
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
        self.pending_wait_cycles += self.clocks.io_8bit_wait_cycles;
        let handled = self.io_write(port, value);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
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

    fn io_read_word(&mut self, port: u16) -> u16 {
        // The IDE data register transfers a full 16-bit word per access;
        // splitting it into byte reads would consume two buffer bytes and
        // spill into the error register.
        if port == 0x01F0 {
            self.pending_wait_cycles += self.clocks.io_16bit_wait_cycles;
            let value = self.ide_read_data_word();
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_IO,
                        TraceAccessKind::Read,
                        u64::from(port),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return value;
        }
        if port == 0x0170 {
            self.pending_wait_cycles += self.clocks.io_16bit_wait_cycles;
            let value = self.ide_secondary_read_data_word();
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_IO,
                        TraceAccessKind::Read,
                        u64::from(port),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return value;
        }
        let low = self.io_read_byte(port) as u16;
        let high = self.io_read_byte(port.wrapping_add(1)) as u16;
        low | (high << 8)
    }

    fn io_write_word(&mut self, port: u16, value: u16) {
        if port == 0x01F0 {
            self.pending_wait_cycles += self.clocks.io_16bit_wait_cycles;
            self.ide_write_data_word(value);
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_IO,
                        TraceAccessKind::Write,
                        u64::from(port),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return;
        }
        if port == 0x0170 {
            self.pending_wait_cycles += self.clocks.io_16bit_wait_cycles;
            self.ide_secondary_write_data_word(value);
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::main_cpu(
                        self.current_cycle,
                        Some(u64::from(self.clocks.cpu_clock_hz)),
                    ),
                    TraceEvent::access(
                        TraceAddressSpace::MAIN_IO,
                        TraceAccessKind::Write,
                        u64::from(port),
                        TraceAccessWidth::Word,
                        Some(u64::from(value)),
                        true,
                    ),
                );
            }
            return;
        }
        self.io_write_byte(port, value as u8);
        self.io_write_byte(port.wrapping_add(1), (value >> 8) as u8);
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.pending_wait_cycles)
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
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::AT_PIC,
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

    fn signal_fpu_error(&mut self) {
        self.fpu_busy_latch = true;
        self.raise_irq(IRQ_FPU);
    }

    fn reset_pending(&self) -> bool {
        self.cpu_reset_pending
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.tracer.yield_requested()
    }

    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;

        if cycle >= self.next_event_cycle {
            self.process_events();
        }
    }
}

#[cfg(test)]
mod tests {
    use common::{
        Bus, TraceAccess, TraceAccessKind, TraceAccessWidth, TraceContext, TraceEvent,
        TraceInterrupt, TraceInterruptAction, TraceSink,
    };

    use super::AtBus;
    use crate::{rom::LoadedRoms, scheduler::EventAt};

    #[derive(Default)]
    struct AccessTrace {
        kinds: Vec<TraceAccessKind>,
        accesses: Vec<TraceAccess>,
        scheduled_contexts: Vec<TraceContext>,
        interrupts: Vec<(TraceContext, TraceInterrupt)>,
    }

    impl TraceSink for AccessTrace {
        fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Access(access) => {
                    self.kinds.push(access.kind);
                    self.accesses.push(access);
                }
                TraceEvent::Scheduled { .. } => self.scheduled_contexts.push(context),
                TraceEvent::Interrupt(interrupt) => self.interrupts.push((context, interrupt)),
                _ => {}
            }
        }
    }

    fn traced_bus() -> AtBus<AccessTrace> {
        let roms = LoadedRoms {
            system_bios: vec![0; 0x1_0000],
            vga_bios: vec![0; 0x8000],
        };
        AtBus::new_with_trace_sink(66_000_000, 0x10_0000, roms, 48_000, AccessTrace::default())
    }

    #[test]
    fn opcode_fetch_is_distinct_from_data_read() {
        let mut bus = traced_bus();

        Bus::read_byte(&mut bus, 0);
        Bus::fetch_opcode_byte(&mut bus, 1);

        assert_eq!(
            bus.tracer().kinds,
            [TraceAccessKind::Read, TraceAccessKind::Fetch]
        );
    }

    #[test]
    fn wide_opcode_fetches_keep_ram_transaction_width() {
        let mut bus = traced_bus();

        Bus::fetch_opcode_word(&mut bus, 0);
        Bus::fetch_opcode_dword(&mut bus, 4);

        assert_eq!(bus.tracer().accesses.len(), 2);
        assert_eq!(bus.tracer().accesses[0].kind, TraceAccessKind::Fetch);
        assert_eq!(bus.tracer().accesses[0].width, TraceAccessWidth::Word);
        assert_eq!(bus.tracer().accesses[1].kind, TraceAccessKind::Fetch);
        assert_eq!(bus.tracer().accesses[1].width, TraceAccessWidth::Dword);
    }

    #[test]
    fn unhandled_io_emits_one_access_with_open_bus_value() {
        let mut bus = traced_bus();

        assert_eq!(Bus::io_read_byte(&mut bus, 0x1234), 0xFF);
        Bus::io_write_byte(&mut bus, 0x1235, 0x66);

        assert_eq!(bus.tracer().accesses.len(), 2);
        assert_eq!(bus.tracer().accesses[0].value, Some(0xFF));
        assert!(!bus.tracer().accesses[0].handled);
        assert_eq!(bus.tracer().accesses[1].value, Some(0x66));
        assert!(!bus.tracer().accesses[1].handled);
    }

    #[test]
    fn scheduler_context_has_explicit_source_and_clock() {
        let mut bus = traced_bus();
        bus.scheduler.schedule(EventAt::KbcDeliver, 7);
        bus.next_event_cycle = 7;

        Bus::set_current_cycle(&mut bus, 7);

        assert_eq!(bus.tracer().scheduled_contexts.len(), 1);
        assert_eq!(
            bus.tracer().scheduled_contexts[0],
            TraceContext::scheduler_main(7, Some(66_000_000))
        );
    }

    #[test]
    fn duplicate_irq_updates_trace_only_transitions() {
        let mut bus = traced_bus();

        bus.raise_irq(5);
        bus.raise_irq(5);
        bus.clear_irq(5);
        bus.clear_irq(5);

        assert_eq!(bus.tracer().interrupts.len(), 2);
        assert_eq!(bus.tracer().interrupts[0].1.line, Some(5));
        assert_eq!(
            bus.tracer().interrupts[0].1.action,
            TraceInterruptAction::Assert
        );
        assert_eq!(bus.tracer().interrupts[1].1.line, Some(5));
        assert_eq!(
            bus.tracer().interrupts[1].1.action,
            TraceInterruptAction::Clear
        );
    }

    #[test]
    fn slave_interrupt_acknowledgement_preserves_global_line() {
        let mut bus = traced_bus();
        bus.pic.write_port0(0, 0x11);
        bus.pic.write_port2(0, 0x08);
        bus.pic.write_port2(0, 0x04);
        bus.pic.write_port2(0, 0x01);
        bus.pic.write_port2(0, 0x00);
        bus.pic.write_port0(1, 0x11);
        bus.pic.write_port2(1, 0x70);
        bus.pic.write_port2(1, 0x02);
        bus.pic.write_port2(1, 0x01);
        bus.pic.write_port2(1, 0x00);
        bus.pic.set_irq(12);

        let vector = Bus::acknowledge_irq(&mut bus);

        assert_eq!(bus.tracer().interrupts.len(), 1);
        let (context, interrupt) = bus.tracer().interrupts[0];
        assert_eq!(context, TraceContext::main_cpu(0, Some(66_000_000)));
        assert_eq!(interrupt.line, Some(12));
        assert_eq!(interrupt.vector, Some(u32::from(vector)));
    }

    struct DisabledPanicTrace;

    impl TraceSink for DisabledPanicTrace {
        const ENABLED: bool = false;

        fn trace(&mut self, _context: TraceContext, _event: TraceEvent<'_>) {
            panic!("disabled tracing callback was invoked");
        }
    }

    #[test]
    fn disabled_sink_is_never_called() {
        let roms = LoadedRoms {
            system_bios: vec![0; 0x1_0000],
            vga_bios: vec![0; 0x8000],
        };
        let mut bus =
            AtBus::new_with_trace_sink(66_000_000, 0x10_0000, roms, 48_000, DisabledPanicTrace);
        Bus::read_byte(&mut bus, 0);
        Bus::fetch_opcode_dword(&mut bus, 1);
        Bus::io_read_byte(&mut bus, 0x1234);
        Bus::io_write_byte(&mut bus, 0x1235, 0x66);
        bus.scheduler.schedule(EventAt::KbcDeliver, 7);
        bus.next_event_cycle = 7;
        Bus::set_current_cycle(&mut bus, 7);
        bus.trace_presentation();
    }
}
