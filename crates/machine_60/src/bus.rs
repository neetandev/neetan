//! PC-6000 system bus.

mod fdc;
mod io_read;
mod io_write;
mod ppi_link;
mod sub_hle;

use std::path::PathBuf;

use common::{
    JoystickState, NoTrace, TraceAccessKind, TraceAccessWidth, TraceAddressSpace, TraceContext,
    TraceEvent, TraceInterruptAction, TraceInterruptKind, TracePresentation, TraceSink, trace_id,
};
use device::{
    ay8910::Ay8910,
    cassette::{CassetteDeck, CassetteError, CassetteRead, parse_tape},
    floppy::FloppyImage,
    i8251_serial::I8251Serial,
    opn_fm::{FmTimerAction, OpnFm, Ym2203},
    upd765a_fdc::{FloppyController, Upd765aFdc},
    upd7752::Upd7752,
};
use fdc::FdcReadState;
use ppi_link::{PpiEffect, PpiLink};
use software_renderer::{
    PC60_MK2_HEIGHT, PC60_MK2_WIDTH, Pc60RenderModel, RenderInputs60, RenderInputsSr, render_pc60,
    render_sr,
};
use sub_hle::SubHle;

use crate::{
    config::{ClockConfig, Pc6000Model},
    interrupt::{InterruptController, IrqSource},
    memory::{BankWindow, Pc6000Memory},
    rom::LoadedRoms,
    scheduler::{Event60, Pc6000Scheduler},
};

/// Open-bus value returned by unmapped I/O reads.
const OPEN_BUS: u8 = 0xFF;

/// PC-6001 base display resolution.
const DISPLAY_WIDTH: u32 = 256;
const DISPLAY_HEIGHT: u32 = 192;
/// PC-6001mkII display resolution.
const MK2_DISPLAY_WIDTH: u32 = PC60_MK2_WIDTH as u32;
const MK2_DISPLAY_HEIGHT: u32 = PC60_MK2_HEIGHT as u32;
const BYTES_PER_PIXEL: usize = 4;

/// AY-3-8910 input clock (main crystal divided by four).
const AY_INPUT_CLOCK_HZ: u32 = 1_996_800;

/// YM2203 input clock on the SR generation (main crystal divided by two).
const OPN_INPUT_CLOCK_HZ: u32 = 3_993_600;

/// Mixing level for the uPD7752 voice relative to the PSG/FM output.
const VOICE_MIX_LEVEL: f32 = 0.4;

/// Internal synthesis rate of the uPD7752, used to pace its frame requests.
const VOICE_SYNTHESIS_RATE_HZ: u64 = 10_000;

/// The PSG/FM device: the discrete AY-3-8910 on the earlier machines, or the
/// YM2203 (OPN) on the SR generation, whose SSG answers the same PSG ports.
enum SoundChip {
    Ay(Ay8910),
    Opn(Box<OpnFm<Ym2203>>),
}

impl Pc6000Bus<NoTrace> {
    /// Creates an untraced bus for `model` at the given audio sample rate.
    pub fn new(model: Pc6000Model, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(model, sample_rate, NoTrace)
    }
}

/// Timer base divider for the periodic interrupt frequency.
const TIMER_BASE_DIVIDER: u64 = 4;
/// Timer base frequency numerator: 487.5 Hz scaled by two to stay integral.
const TIMER_BASE_FREQ_X2: u64 = 975;
/// Power-on timer rate divider.
const TIMER_DEFAULT_HZ_DIV: u64 = 3;

/// Vertical retrace / frame rate.
const VRTC_HZ: u64 = 60;
/// Scanlines per frame (including blanking).
const LINES_PER_FRAME: u64 = 262;
/// Active display lines that steal the bus from the CPU.
const ACTIVE_DISPLAY_LINES: u16 = 192;
/// Fraction of each active scanline the video circuit holds the bus: the CPU is
/// stalled for `BUSREQ_NUMERATOR / BUSREQ_DENOMINATOR` of the line period.
const BUSREQ_NUMERATOR: u64 = 296;
const BUSREQ_DENOMINATOR: u64 = 455;

/// Memory wait states (in main-clock cycles) added per access.
const MEMORY_WAIT_CYCLES: i64 = 1;
/// Below this address a memory access incurs a wait state.
const MEMORY_WAIT_LIMIT: u32 = 0x8000;
/// I/O ports in this 16-port block (0xA0-0xAF, the PSG/FM) incur a wait state.
const IO_WAIT_BLOCK: u16 = 0xA0;
/// Keyboard scan rate.
const KEY_SCAN_HZ: u64 = 250;
/// Cassette byte delivery rate (1200 baud over a twelve-bit frame).
const CASSETTE_BYTE_HZ: u64 = 100;

/// Sub-controller command that triggers a joystick interrupt.
const SUB_COMMAND_JOYSTICK_TRIGGER: u8 = 0x06;
/// Sub-controller command that starts the cassette transport.
const SUB_COMMAND_CASSETTE_PLAY: u8 = 0x19;
/// Sub-controller command that stops the cassette transport.
const SUB_COMMAND_CASSETTE_STOP: u8 = 0x1A;

/// Built-in FDC data rate: 250 kbit/s MFM is 31250 bytes/s.
const FDC_DATA_RATE_BYTES_PER_SEC: u64 = 31_250;
/// Forced-ready control bit on the uPD765A: the built-in drive presents as ready
/// while its motor is driven, so a data command on an empty drive fails on a
/// missing address mark rather than reporting "not ready".
const FDC_FORCED_READY: u8 = 0x40;

/// System-latch bit that drives the cassette motor (PLAY when set).
const SYSTEM_LATCH_CASSETTE_MOTOR: u8 = 0x08;
/// Sub-CPU vector raised when a cassette data byte is ready.
const CASSETTE_DATA_VECTOR: u8 = 0x08;
/// Sub-CPU vector raised at the end of the tape.
const CASSETTE_END_VECTOR: u8 = 0x12;

/// Video RAM base addresses selected by system-latch bits [2:1].
const VIDEO_RAM_BASES: [u16; 4] = [0xC000, 0xE000, 0x8000, 0xA000];

/// mkII video base offsets into work RAM, selected by the combined VRAM bank
/// and system-latch bits.
const MK2_VIDEO_BASE_OFFSETS: [u16; 8] = [
    0x8000, 0xC000, 0xC000, 0xE000, 0x0000, 0x8000, 0x4000, 0xA000,
];

/// PC-6000 system bus.
pub struct Pc6000Bus<T: TraceSink = NoTrace> {
    model: Pc6000Model,
    clocks: ClockConfig,
    memory: Pc6000Memory,
    /// Event scheduler.
    pub(crate) scheduler: Pc6000Scheduler,
    interrupt: InterruptController,
    ppi: PpiLink,
    sub: SubHle,
    sound: SoundChip,
    voice: Upd7752,
    /// SR serial port (ports 0x80-0x81); always TX-ready so polling never hangs.
    serial: I8251Serial,
    cassette: CassetteDeck,
    cassette_active: bool,
    /// Built-in non-intelligent uPD765A, driven directly by the main CPU.
    fdc: Upd765aFdc,
    /// Mounted floppy drives backing the built-in FDC.
    floppy: FloppyController,
    /// Whether the built-in drive motor is running.
    fdc_motor_on: bool,
    /// Port 0xB1 bit 2: the external intelligent unit is selected. The external
    /// path is unimplemented, so the built-in FDC only answers while this is clear.
    fdc_external_selected: bool,
    /// Main-clock cycles between non-DMA byte transfers at the FDC data rate.
    fdc_drq_byte_cycles: u64,
    /// Read-path state for deleted-mark / CRC / READ TRACK handling.
    fdc_read: FdcReadState,
    /// Memory/I/O wait cycles accrued since the CPU last drained them.
    memory_wait_cycles: i64,
    /// Whether the video circuit currently holds the bus (CPU stalled).
    busreq_active: bool,
    /// Current scanline counter (0..LINES_PER_FRAME).
    scanline: u16,
    /// Main-clock cycles per scanline.
    line_period: u64,
    /// Main-clock cycles the bus is held per active scanline.
    busreq_window: u64,
    current_cycle: u64,
    timer_enabled: bool,
    timer_irq_masked: bool,
    timer_hz_div: u64,
    joystick_directions: u8,
    system_latch: u8,
    bgcol_bank: u8,
    ex_vram_bank: u8,
    exgfx_bitmap: bool,
    exgfx_2bpp: bool,
    exgfx_text: bool,
    sr_text_mode: bool,
    sr_text_rows: u8,
    sr_width80: bool,
    sr_compat: bool,
    sr_scroll_x: u16,
    sr_scroll_y: u8,
    sr_bitmap_x_offset: u8,
    sr_bitmap_y_offset: u8,
    framebuffer: Vec<u8>,
    presented_frames: u64,
    /// Bus-activity tracer (a no-op by default).
    tracer: T,
}

impl<T: TraceSink> Pc6000Bus<T> {
    /// Creates a traced bus for `model` at the given audio sample rate.
    pub fn new_with_trace_sink(model: Pc6000Model, sample_rate: u32, tracer: T) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sample_rate,
        };
        let (width, height) = display_dimensions(model);
        let mut bus = Self {
            model,
            clocks,
            memory: Pc6000Memory::new(model),
            scheduler: Pc6000Scheduler::new(),
            interrupt: InterruptController::new(model.is_sr()),
            ppi: PpiLink::new(),
            sub: SubHle::new(),
            sound: if model.has_fm() {
                SoundChip::Opn(Box::new(OpnFm::new(
                    model.main_clock_hz(),
                    sample_rate,
                    OPN_INPUT_CLOCK_HZ,
                )))
            } else {
                SoundChip::Ay(Ay8910::new())
            },
            voice: Upd7752::new(sample_rate),
            serial: I8251Serial::new(),
            cassette: CassetteDeck::new(),
            cassette_active: false,
            fdc: Upd765aFdc::new(),
            floppy: FloppyController::new(),
            fdc_motor_on: false,
            fdc_external_selected: false,
            fdc_drq_byte_cycles: (u64::from(model.main_clock_hz()) / FDC_DATA_RATE_BYTES_PER_SEC)
                .max(1),
            fdc_read: FdcReadState::default(),
            memory_wait_cycles: 0,
            busreq_active: false,
            scanline: 0,
            line_period: (u64::from(model.main_clock_hz()) / VRTC_HZ / LINES_PER_FRAME).max(1),
            busreq_window: u64::from(model.main_clock_hz()) / VRTC_HZ / LINES_PER_FRAME
                * BUSREQ_NUMERATOR
                / BUSREQ_DENOMINATOR,
            current_cycle: 0,
            timer_enabled: false,
            timer_irq_masked: false,
            timer_hz_div: TIMER_DEFAULT_HZ_DIV,
            joystick_directions: 0x3F,
            system_latch: 0,
            bgcol_bank: 0,
            ex_vram_bank: 0,
            exgfx_bitmap: false,
            exgfx_2bpp: false,
            exgfx_text: false,
            sr_text_mode: true,
            sr_text_rows: 20,
            sr_width80: false,
            sr_compat: false,
            sr_scroll_x: 0,
            sr_scroll_y: 0,
            sr_bitmap_x_offset: 0,
            sr_bitmap_y_offset: 0,
            framebuffer: vec![0; width as usize * height as usize * BYTES_PER_PIXEL],
            presented_frames: 0,
            tracer,
        };
        bus.schedule_frame();
        bus.schedule_key_scan();
        bus.scheduler.schedule(Event60::Scanline, bus.line_period);
        bus
    }

    /// Applies a loaded ROM set. The base CG and BASIC ROM are required; the
    /// extended CG, voice and kanji ROMs are loaded into the banked map when the
    /// model provides them.
    pub fn load_roms(&mut self, roms: &LoadedRoms) {
        if self.model.is_sr() {
            let half1 = roms.system_rom1.as_deref().unwrap_or(&[]);
            let half2 = roms.system_rom2.as_deref().unwrap_or(&[]);
            self.memory.load_sr_system_rom(half1, half2);
            if let Some(cg) = roms.cg_sr.as_deref() {
                self.memory.load_cgrom(cg);
            }
            if let Some(cg) = roms.cg_base.as_deref() {
                self.memory.load_sr_compat_cgrom(cg);
            }
            if let Some(ext) = roms.cg_ext.as_deref() {
                self.memory.load_ext_cgrom(ext);
            }
            if let Some(voice) = roms.voice.as_deref() {
                self.memory.load_voice_rom(voice);
            }
            if let Some(kanji) = roms.kanji.as_deref() {
                self.memory.load_kanji_rom(kanji);
            }
            return;
        }
        self.memory.load_basic_rom(roms.boot_rom());
        self.memory.load_cgrom(roms.font_rom());
        if let Some(ext) = roms.cg_ext.as_deref() {
            self.memory.load_ext_cgrom(ext);
        }
        if let Some(voice) = roms.voice.as_deref() {
            self.memory.load_voice_rom(voice);
        }
        if let Some(kanji) = roms.kanji.as_deref() {
            self.memory.load_kanji_rom(kanji);
        }
    }

    /// Loads a cartridge image into the cartridge slot.
    pub fn load_cartridge(&mut self, image: &[u8]) {
        self.memory.load_cartridge(image);
    }

    /// Parses a cassette image (chosen by file extension) and loads it into the
    /// deck, leaving it stopped. The guest drives the motor via the system latch.
    pub fn insert_cassette(&mut self, extension: &str, image: &[u8]) -> Result<(), CassetteError> {
        let tape = parse_tape(extension, image)?;
        self.cassette.insert(tape);
        Ok(())
    }

    /// Stops and removes the loaded cassette.
    pub fn eject_cassette(&mut self) {
        self.cassette.eject();
        self.cassette_active = false;
        self.scheduler.cancel(Event60::CassetteByte);
    }

    /// Mounts a floppy image into a drive and marks the FDC drive as occupied.
    pub fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: Option<PathBuf>) {
        self.floppy.insert_drive(drive, image, path);
        if drive < 4 {
            self.fdc.state.drive_has_disk |= 1 << drive;
        }
    }

    /// Ejects the floppy from a drive and clears the FDC drive-occupied bit.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.floppy.eject_drive(drive);
        if drive < 4 {
            self.fdc.state.drive_has_disk &= !(1 << drive);
        }
    }

    /// Flushes any dirty mounted floppies back to their source files.
    pub fn flush_floppies(&mut self) {
        self.floppy.flush_all_drives();
    }

    /// The configured machine model.
    pub fn model(&self) -> Pc6000Model {
        self.model
    }

    /// Main CPU clock frequency in Hz.
    pub fn cpu_clock_hz(&self) -> u32 {
        self.clocks.main_clock_hz
    }

    /// The current monotonic cycle count (main-clock units).
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// A shared reference to the bus-activity tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// A mutable reference to the bus-activity tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Advances the monotonic cycle counter.
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    /// The cycle of the next scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Reads a byte through the active memory map (for tests and tooling).
    pub fn peek_byte(&self, address: u16) -> u8 {
        self.memory.read(address)
    }

    /// Writes a byte through the active memory map (for tests and tooling).
    pub fn poke_byte(&mut self, address: u16, value: u8) {
        self.memory.write(address, value);
    }

    /// The last rendered framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// The framebuffer dimensions.
    pub fn display_dimensions(&self) -> (u32, u32) {
        display_dimensions(self.model)
    }

    /// The character generator ROM data.
    pub fn font_rom_data(&self) -> &[u8] {
        self.memory.cgrom()
    }

    /// Injects a host keyboard event (bit 7 set marks a release).
    pub fn push_keyboard_scancode(&mut self, code: u8) {
        self.sub.push_scancode(code);
    }

    /// Updates the joystick state read through the PSG port A.
    pub fn set_joystick(&mut self, state: JoystickState) {
        // Active-low: a pressed direction or button clears its bit.
        let mut value = 0x3Fu8;
        let mut clear = |pressed: bool, bit: u8| {
            if pressed {
                value &= !(1 << bit);
            }
        };
        clear(state.up, 0);
        clear(state.down, 1);
        clear(state.left, 2);
        clear(state.right, 3);
        clear(state.trigger1, 4);
        clear(state.trigger2, 5);
        self.joystick_directions = value;
    }

    /// Generates audio for the elapsed frame, returning the number of `f32`
    /// values written.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let current_cycle = self.current_cycle;
        let main_clock_hz = self.clocks.main_clock_hz;
        let sample_rate = self.clocks.sample_rate;
        let written = match &mut self.sound {
            SoundChip::Ay(ay) => ay.generate_samples(
                current_cycle,
                AY_INPUT_CLOCK_HZ,
                main_clock_hz,
                sample_rate,
                volume,
                output,
            ),
            SoundChip::Opn(opn) => {
                opn.generate_samples(current_cycle, main_clock_hz, volume, output);
                output.len()
            }
        };
        // The voice synthesizer mixes on top of the PSG/FM output.
        self.voice
            .mix_into(&mut output[..written], volume * VOICE_MIX_LEVEL);
        self.apply_fm_timers();
        written
    }

    /// Reads a PSG/FM port (0xA0-0xA3).
    pub(super) fn psg_read(&mut self, port: u16) -> u8 {
        let joystick = self.joystick_port_a();
        let current_cycle = self.current_cycle;
        match &mut self.sound {
            SoundChip::Ay(ay) => {
                if port & 0x03 == 0x02 {
                    ay.set_port_a_input(joystick);
                    ay.data_r()
                } else {
                    OPEN_BUS
                }
            }
            SoundChip::Opn(opn) => {
                opn.set_io_input(0, joystick);
                // 0xA0/0xA3 read the YM2203 status register (bit 7 BUSY); 0xA1/0xA2
                // read the selected register's data. The SR firmware's vertical
                // retrace handler polls the BUSY bit on 0xA3 before touching the
                // SSG, so 0xA3 must answer with status, not data.
                match port & 0x03 {
                    0 | 3 => opn.read_status(current_cycle),
                    _ => opn.read_data(current_cycle),
                }
            }
        }
    }

    /// Writes a PSG/FM port (0xA0-0xA3).
    pub(super) fn psg_write(&mut self, port: u16, value: u8) {
        let current_cycle = self.current_cycle;
        match &mut self.sound {
            SoundChip::Ay(ay) => match port & 0x03 {
                0 => ay.address_w(value),
                1 => ay.data_w(value),
                _ => {}
            },
            SoundChip::Opn(opn) => match port & 0x03 {
                0 => opn.write_address(value, current_cycle),
                1 => opn.write_data(value, current_cycle),
                _ => {}
            },
        }
        self.apply_fm_timers();
    }

    /// Drains the YM2203 FM timer requests onto the scheduler. The YM2203 /IRQ
    /// pin is not wired to the CPU on the PC-6001, so its timer overflow is only
    /// observable through the status register, never as an interrupt; the
    /// overflow flag is drained here so the chip state stays consistent.
    fn apply_fm_timers(&mut self) {
        let actions = {
            let SoundChip::Opn(opn) = &mut self.sound else {
                return;
            };
            let mut actions = [None::<FmTimerAction>, None];
            for (slot, action) in actions.iter_mut().zip(opn.drain_timers().iter()) {
                *slot = Some(*action);
            }
            let _ = opn.take_irq_change();
            actions
        };
        for action in actions.into_iter().flatten() {
            match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => self
                    .scheduler
                    .schedule(fm_timer_event(timer_id), fire_cycle),
                FmTimerAction::Cancel { timer_id } => {
                    self.scheduler.cancel(fm_timer_event(timer_id))
                }
            }
        }
    }

    /// Writes a uPD7752 voice register and refreshes its interrupt line. The
    /// chip raises the voice interrupt while it is busy in external-message mode
    /// and waiting for the next parameter byte; the SR voice driver feeds frames
    /// from that interrupt rather than blocking.
    fn voice_write(&mut self, offset: u8, value: u8) {
        self.voice.write(offset, value);
        self.after_voice_change();
    }

    /// Paces the uPD7752 data-request line after a register write. When the chip
    /// finishes a frame it drops the request until that frame has played; this
    /// schedules the re-request for the correct playback delay and refreshes the
    /// voice interrupt so the SR driver is called back exactly when a new frame
    /// is due.
    fn after_voice_change(&mut self) {
        if let Some(samples) = self.voice.pending_request_samples() {
            let delay =
                u64::from(self.clocks.main_clock_hz) * samples as u64 / VOICE_SYNTHESIS_RATE_HZ;
            self.scheduler
                .schedule(Event60::VoiceRequest, self.current_cycle + delay);
        }
        self.update_voice_irq();
    }

    /// Raises or clears the voice interrupt from the uPD7752 request line. Only
    /// the SR generation drives the synthesizer from this interrupt; the earlier
    /// machines feed it with a blocking polled loop and leave the voice interrupt
    /// masked, so a raise there would vector into a handler the polled driver
    /// never set up.
    fn update_voice_irq(&mut self) {
        if !self.model.is_sr() {
            return;
        }
        if self.voice.wants_data() {
            self.interrupt.raise(IrqSource::Voice);
        } else {
            self.interrupt.clear(IrqSource::Voice);
        }
    }

    /// Notifies the YM2203 that an FM timer has expired.
    fn fm_timer_expired(&mut self, timer_id: u32) {
        let current_cycle = self.current_cycle;
        if let SoundChip::Opn(opn) = &mut self.sound {
            opn.timer_expired(timer_id, current_cycle);
        }
        self.apply_fm_timers();
    }

    /// Processes any scheduler events due at the current cycle.
    pub fn process_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        for event in due.iter() {
            if T::ENABLED {
                self.tracer.trace(
                    TraceContext::scheduler_main(
                        self.current_cycle,
                        Some(u64::from(self.cpu_clock_hz())),
                    ),
                    TraceEvent::Scheduled {
                        event: event.kind.trace_name(),
                        fire_tick: event.fire_cycle,
                    },
                );
            }
            match event.kind {
                Event60::TimerIrq => {
                    if self.timer_enabled && !self.timer_irq_masked && !self.cassette_active {
                        self.interrupt.raise(IrqSource::Timer);
                    }
                    self.schedule_timer();
                }
                Event60::Vrtc => {
                    if self.model.is_sr() && !self.sr_compat {
                        self.interrupt.raise(IrqSource::Vrtc);
                    }
                    self.render_frame();
                    self.trace_presentation();
                    self.schedule_frame();
                }
                Event60::KeyScan => {
                    // The cassette byte and the keyboard share the port A latch,
                    // so the host only scans keys while the tape is stopped.
                    if !self.cassette_active
                        && let Some(vector) = self.sub.scan()
                    {
                        self.interrupt.set_sub_vector(vector);
                    }
                    self.schedule_key_scan();
                }
                Event60::CassetteByte => self.deliver_cassette_byte(),
                Event60::FdcDrqByte => self.on_fdc_drq_byte(),
                Event60::FdcSeekComplete => self.fdc.state.interrupt_pending = true,
                Event60::FmTimerA => self.fm_timer_expired(0),
                Event60::FmTimerB => self.fm_timer_expired(1),
                Event60::Scanline => self.on_scanline(),
                Event60::BusReqEnd => self.busreq_active = false,
                Event60::VoiceRequest => {
                    self.voice.arm_request();
                    self.update_voice_irq();
                }
            }
        }
    }

    /// Advances the scanline counter, asserting the VRAM bus-request stall for
    /// the active-display portion of each line while the CRT is enabled.
    fn on_scanline(&mut self) {
        let line = self.scanline;
        self.scanline = (self.scanline + 1) % LINES_PER_FRAME as u16;
        if line < ACTIVE_DISPLAY_LINES && self.ppi.crt_enabled() && self.busreq_window > 0 {
            self.busreq_active = true;
            self.scheduler
                .schedule(Event60::BusReqEnd, self.current_cycle + self.busreq_window);
        }
        self.scheduler
            .schedule(Event60::Scanline, self.current_cycle + self.line_period);
    }

    /// Whether the CPU is currently held off the bus by the video circuit.
    pub fn cpu_stalled(&self) -> bool {
        self.busreq_active
    }

    /// Releases the video bus-request, returning the bus to the CPU.
    fn release_busreq(&mut self) {
        self.busreq_active = false;
        self.scheduler.cancel(Event60::BusReqEnd);
    }

    /// Whether an interrupt is pending for the CPU.
    pub fn has_irq(&self) -> bool {
        self.interrupt.has_pending()
    }

    /// Acknowledges the highest-priority pending interrupt and reports its source.
    pub fn acknowledge_irq(&mut self) -> crate::interrupt::InterruptAcknowledge {
        self.interrupt.acknowledge()
    }

    fn trace_presentation(&mut self) {
        if !T::ENABLED {
            return;
        }
        let (width, height) = display_dimensions(self.model);
        self.presented_frames = self.presented_frames.saturating_add(1);
        self.tracer.trace(
            TraceContext::presentation_main(
                self.current_cycle,
                Some(u64::from(self.cpu_clock_hz())),
            ),
            TraceEvent::Presentation(TracePresentation {
                display: trace_id::display::MAIN,
                frame: self.presented_frames,
                width,
                height,
            }),
        );
    }

    fn render_frame(&mut self) {
        match self.model {
            Pc6000Model::Pc6001 => self.render_legacy_frame(Pc60RenderModel::Base),
            Pc6000Model::Pc6001Mk2 | Pc6000Model::Pc6601 => {
                self.render_legacy_frame(Pc60RenderModel::Mk2)
            }
            Pc6000Model::Pc6001Mk2Sr | Pc6000Model::Pc6601Sr => self.render_sr_frame(),
        }
    }

    /// Renders a base PC-6001 or mkII frame (also the SR mkII-compatibility path).
    fn render_legacy_frame(&mut self, model: Pc60RenderModel) {
        if self.model.is_sr() && self.sr_compat {
            let Some(sr) = self.memory.sr() else {
                return;
            };
            let inputs = RenderInputs60 {
                model,
                vram: sr.legacy_video_window(),
                cgrom: sr.compat_cgrom(),
                exgfx_bitmap: self.exgfx_bitmap,
                exgfx_2bpp: self.exgfx_2bpp,
                exgfx_text: self.exgfx_text,
                bgcol_bank: self.bgcol_bank,
            };
            render_pc60(&inputs, &mut self.framebuffer);
            return;
        }
        let inputs = RenderInputs60 {
            model,
            vram: self.memory.video_ram(),
            cgrom: self.memory.cgrom(),
            exgfx_bitmap: self.exgfx_bitmap,
            exgfx_2bpp: self.exgfx_2bpp,
            exgfx_text: self.exgfx_text,
            bgcol_bank: self.bgcol_bank,
        };
        render_pc60(&inputs, &mut self.framebuffer);
    }

    /// Renders a native SR frame, or routes through the legacy renderer when the
    /// mkII-compatibility bit is set.
    fn render_sr_frame(&mut self) {
        if self.sr_compat {
            self.render_legacy_frame(Pc60RenderModel::Mk2);
            return;
        }
        let Some(sr) = self.memory.sr() else {
            return;
        };
        let inputs = RenderInputsSr {
            vram: sr.text_window(),
            cgrom: sr.cgrom(),
            gvram: sr.gvram(),
            text_mode: self.sr_text_mode,
            text_rows: self.sr_text_rows,
            width80: self.sr_width80,
            scroll_x: self.sr_scroll_x,
            scroll_y: self.sr_scroll_y,
        };
        render_sr(&inputs, &mut self.framebuffer);
    }

    /// Applies the SR video mode register (port 0xC8).
    fn set_sr_mode_register(&mut self, value: u8) {
        self.sr_text_mode = value & 0x08 != 0;
        self.sr_text_rows = if value & 0x04 != 0 { 20 } else { 25 };
        self.sr_compat = value & 0x01 != 0;
        if let Some(sr) = self.memory.sr_mut() {
            sr.set_bitmap_mode(!self.sr_text_mode);
            if self.sr_compat {
                sr.apply_compat_write_bank();
            }
        }
        if self.sr_compat {
            self.interrupt.clear(IrqSource::Voice);
            self.interrupt.clear(IrqSource::Vrtc);
            self.recompute_mk2_video_base();
        }
    }

    /// Sets the SR scroll registers (ports 0xCA-0xCC).
    fn set_sr_scroll(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            0xCA => self.sr_scroll_x = (self.sr_scroll_x & 0xFF00) | u16::from(value),
            0xCB => self.sr_scroll_x = (self.sr_scroll_x & 0x00FF) | (u16::from(value) << 8),
            _ => self.sr_scroll_y = value,
        }
    }

    /// Sets an SR bitmap offset register (ports 0xCE/0xCF) and forwards both to
    /// the graphics VRAM overlay.
    fn set_sr_bitmap_offset(&mut self, port: u16, value: u8) {
        match port & 0xFF {
            0xCE => self.sr_bitmap_y_offset = value,
            _ => self.sr_bitmap_x_offset = value,
        }
        if let Some(sr) = self.memory.sr_mut() {
            sr.set_bitmap_offsets(self.sr_bitmap_x_offset, self.sr_bitmap_y_offset);
        }
    }

    fn timer_period(&self) -> u64 {
        let clock = u64::from(self.clocks.main_clock_hz);
        // period = clock / (487.5 * base / (hz_div + 1))
        let denominator = TIMER_BASE_FREQ_X2 * TIMER_BASE_DIVIDER;
        (clock * 2 * (self.timer_hz_div + 1) / denominator).max(1)
    }

    fn frame_period(&self) -> u64 {
        (u64::from(self.clocks.main_clock_hz) / VRTC_HZ).max(1)
    }

    fn key_scan_period(&self) -> u64 {
        (u64::from(self.clocks.main_clock_hz) / KEY_SCAN_HZ).max(1)
    }

    fn cassette_byte_period(&self) -> u64 {
        (u64::from(self.clocks.main_clock_hz) / CASSETTE_BYTE_HZ).max(1)
    }

    fn schedule_cassette_byte(&mut self) {
        let fire = self.current_cycle + self.cassette_byte_period();
        self.scheduler.schedule(Event60::CassetteByte, fire);
    }

    /// Delivers the next cassette byte to the sub-CPU latch and raises the
    /// data-ready interrupt, or signals end of tape and stops the transport.
    fn deliver_cassette_byte(&mut self) {
        match self.cassette.read_byte() {
            CassetteRead::Byte(byte) => {
                self.sub.set_cassette_byte(byte);
                self.interrupt.set_sub_vector(CASSETTE_DATA_VECTOR);
                self.schedule_cassette_byte();
            }
            CassetteRead::EndOfTape => {
                self.interrupt.set_sub_vector(CASSETTE_END_VECTOR);
                self.cassette_active = false;
                self.cassette.set_motor(false);
            }
        }
    }

    fn schedule_timer(&mut self) {
        if self.timer_enabled {
            let fire = self.current_cycle + self.timer_period();
            self.scheduler.schedule(Event60::TimerIrq, fire);
        } else {
            self.scheduler.cancel(Event60::TimerIrq);
        }
    }

    fn schedule_frame(&mut self) {
        let fire = self.current_cycle + self.frame_period();
        self.scheduler.schedule(Event60::Vrtc, fire);
    }

    fn schedule_key_scan(&mut self) {
        let fire = self.current_cycle + self.key_scan_period();
        self.scheduler.schedule(Event60::KeyScan, fire);
    }

    fn system_latch_write(&mut self, data: u8) {
        self.system_latch = data;
        match self.model {
            Pc6000Model::Pc6001 => {
                let base = VIDEO_RAM_BASES[((data >> 1) & 0x03) as usize];
                self.memory.set_video_ram_base(base);
            }
            Pc6000Model::Pc6001Mk2
            | Pc6000Model::Pc6601
            | Pc6000Model::Pc6001Mk2Sr
            | Pc6000Model::Pc6601Sr => self.recompute_mk2_video_base(),
        }

        let enable = data & 1 == 0;
        if enable != self.timer_enabled {
            self.timer_enabled = enable;
            self.schedule_timer();
        }

        self.set_cassette_motor(data & SYSTEM_LATCH_CASSETTE_MOTOR != 0);
    }

    /// Recomputes the mkII video base from the VRAM bank and system-latch bits.
    fn recompute_mk2_video_base(&mut self) {
        let vram_bank = (self.ex_vram_bank & 0x06) | ((self.system_latch & 0x06) << 4);
        let index = (((vram_bank & 0x60) >> 4) | ((vram_bank & 0x02) >> 1)) as usize;
        let offset = MK2_VIDEO_BASE_OFFSETS[index] as usize;
        if let Some(banked) = self.memory.banked_mut() {
            banked.set_video_base(offset);
        } else if let Some(sr) = self.memory.sr_mut() {
            sr.set_legacy_video_base(offset);
        }
    }

    /// Applies the mkII video mode register (port 0xC1): selects the extended
    /// modes, the character-generator half exposed by the gfx bank, and the
    /// VRAM bank.
    fn set_video_mode_register(&mut self, value: u8) {
        self.ex_vram_bank = value;
        self.exgfx_text = value & 0x02 == 0;
        self.exgfx_bitmap = value & 0x08 != 0;
        self.exgfx_2bpp = value & 0x06 == 0;
        let cgrom_bank_addr = if value & 0x02 != 0 { 0x0000 } else { 0x2000 };
        if let Some(banked) = self.memory.banked_mut() {
            banked.set_cgrom_bank_addr(cgrom_bank_addr);
        } else if let Some(sr) = self.memory.sr_mut() {
            sr.set_compat_cgrom_bank_addr(cgrom_bank_addr);
        }
        self.recompute_mk2_video_base();
    }

    /// Sets the timer divider (mkII port 0xF6) and reschedules the next tick.
    fn set_timer_divider(&mut self, value: u8) {
        self.timer_hz_div = u64::from(value);
        self.schedule_timer();
    }

    /// Drives the cassette motor from the system latch. A stop-to-play edge
    /// starts byte delivery; a play-to-stop edge halts it.
    fn set_cassette_motor(&mut self, on: bool) {
        if on == self.cassette_active {
            return;
        }
        self.cassette_active = on;
        self.cassette.set_motor(on);
        if on {
            self.schedule_cassette_byte();
        } else {
            self.scheduler.cancel(Event60::CassetteByte);
        }
    }

    /// Joystick byte read through PSG port A: direction/button bits plus the
    /// horizontal and vertical sync flags (active-low).
    fn joystick_port_a(&self) -> u8 {
        let phase = self.current_cycle % self.frame_period();
        let vblank = phase * 100 >= self.frame_period() * 73;
        let mut value = self.joystick_directions | 0x40;
        if vblank {
            value &= 0x7F;
        } else {
            value |= 0x80;
        }
        value
    }

    fn apply_ppi_effect(&mut self, effect: PpiEffect) {
        match effect {
            PpiEffect::SetBank(window) => self.apply_bank_window(window),
            PpiEffect::SubCommand(SUB_COMMAND_JOYSTICK_TRIGGER) => {
                self.interrupt.raise(IrqSource::Joystick);
            }
            PpiEffect::SubCommand(SUB_COMMAND_CASSETTE_PLAY) => self.set_cassette_motor(true),
            PpiEffect::SubCommand(SUB_COMMAND_CASSETTE_STOP) => self.set_cassette_motor(false),
            PpiEffect::SubCommand(_) | PpiEffect::None => {}
        }
    }

    /// Applies a bank-window control word. On the base machine it swaps the
    /// 0x6000 window between the cartridge and the character generator; on the
    /// banked machines it toggles the character-generator gfx bank.
    fn apply_bank_window(&mut self, window: BankWindow) {
        match self.model {
            Pc6000Model::Pc6001 => {
                if window == BankWindow::CartridgeUpper && !self.memory.has_cartridge() {
                    return;
                }
                self.memory.set_bank_window(window);
            }
            Pc6000Model::Pc6001Mk2
            | Pc6000Model::Pc6601
            | Pc6000Model::Pc6001Mk2Sr
            | Pc6000Model::Pc6601Sr => {
                if let Some(banked) = self.memory.banked_mut() {
                    banked.set_gfx_bank(window == BankWindow::CharacterGenerator);
                } else if self.sr_compat
                    && let Some(sr) = self.memory.sr_mut()
                {
                    sr.set_compat_gfx_bank(window == BankWindow::CharacterGenerator);
                }
            }
        }
    }
}

/// Maps a YM2203 timer id (0 = A, 1 = B) to its scheduler event.
fn fm_timer_event(timer_id: u8) -> Event60 {
    if timer_id == 0 {
        Event60::FmTimerA
    } else {
        Event60::FmTimerB
    }
}

/// The framebuffer dimensions for a model.
fn display_dimensions(model: Pc6000Model) -> (u32, u32) {
    match model {
        Pc6000Model::Pc6001 => (DISPLAY_WIDTH, DISPLAY_HEIGHT),
        Pc6000Model::Pc6001Mk2
        | Pc6000Model::Pc6601
        | Pc6000Model::Pc6001Mk2Sr
        | Pc6000Model::Pc6601Sr => (MK2_DISPLAY_WIDTH, MK2_DISPLAY_HEIGHT),
    }
}

/// Wait cycles for a data memory access: the low ROM/RAM region is one cycle
/// slower than the upper half.
fn memory_wait(address: u32) -> i64 {
    if address & 0xFFFF < MEMORY_WAIT_LIMIT {
        MEMORY_WAIT_CYCLES
    } else {
        0
    }
}

/// Wait cycles for an I/O access: the PSG/FM port block costs one extra cycle.
fn io_wait(port: u16) -> i64 {
    if port & 0xF0 == IO_WAIT_BLOCK {
        MEMORY_WAIT_CYCLES
    } else {
        0
    }
}

/// Ephemeral `common::Bus` adapter for the main Z80.
pub struct MainBusView<'a, T: TraceSink = NoTrace> {
    pub bus: &'a mut Pc6000Bus<T>,
}

impl<T: TraceSink> common::Bus for MainBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        self.bus.memory_wait_cycles += memory_wait(address);
        let bus_address = address as u16;
        let value = self.bus.memory.read(bus_address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Read,
                    u64::from(bus_address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        self.bus.memory_wait_cycles += memory_wait(address);
        let bus_address = address as u16;
        self.bus.memory.write(bus_address, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(bus_address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        // An M1 opcode fetch always costs one wait state, independent of the
        // address (it replaces the data-access wait rather than adding to it).
        self.bus.memory_wait_cycles += MEMORY_WAIT_CYCLES;
        let value = self.bus.memory.read(address as u16);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Fetch,
                    u64::from(address as u16),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        self.bus.memory_wait_cycles += io_wait(port);
        let (value, handled) = self.bus.io_read(port);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
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
        self.bus.memory_wait_cycles += io_wait(port);
        let handled = self.bus.io_write(port, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
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

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.bus.memory_wait_cycles)
    }

    fn has_irq(&self) -> bool {
        self.bus.has_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let acknowledge = self.bus.acknowledge_irq();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::PC60_IRQ,
                    TraceInterruptKind::Maskable,
                    acknowledge.source.map(|source| source as u16),
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
        self.bus.current_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.bus.current_cycle = cycle;
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.bus.tracer.yield_requested()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Pc6000Model;

    #[derive(Default)]
    struct RecordingTrace {
        accesses: Vec<common::TraceAccess>,
        interrupts: Vec<common::TraceInterrupt>,
    }

    impl TraceSink for RecordingTrace {
        fn trace(&mut self, _context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Access(access) => self.accesses.push(access),
                TraceEvent::Interrupt(interrupt) => self.interrupts.push(interrupt),
                _ => {}
            }
        }
    }

    /// System-latch byte that enables the cassette motor while keeping the timer
    /// disabled (bit 0 set), to isolate cassette interrupts.
    const MOTOR_ON_TIMER_OFF: u8 = SYSTEM_LATCH_CASSETTE_MOTOR | 0x01;

    fn fire_next_event(bus: &mut Pc6000Bus) -> Option<u8> {
        let next = bus
            .scheduler
            .next_event_cycle()
            .expect("an event is scheduled");
        bus.set_current_cycle(next);
        bus.process_events();
        bus.has_irq().then(|| bus.acknowledge_irq().vector)
    }

    #[test]
    fn cassette_delivers_bytes_then_end_of_tape() {
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        // Isolate the cassette path from the frame, key-scan and scanline events.
        bus.scheduler.cancel(Event60::Vrtc);
        bus.scheduler.cancel(Event60::KeyScan);
        bus.scheduler.cancel(Event60::Scanline);
        bus.insert_cassette("p6", &[0xA1, 0xB2])
            .expect("tape parses");

        bus.system_latch_write(MOTOR_ON_TIMER_OFF);

        assert_eq!(fire_next_event(&mut bus), Some(CASSETTE_DATA_VECTOR));
        assert_eq!(bus.sub.current_keycode(), 0xA1);
        assert_eq!(fire_next_event(&mut bus), Some(CASSETTE_DATA_VECTOR));
        assert_eq!(bus.sub.current_keycode(), 0xB2);
        assert_eq!(fire_next_event(&mut bus), Some(CASSETTE_END_VECTOR));

        assert!(!bus.cassette_active);
        assert_eq!(bus.scheduler.next_event_cycle(), None);
    }

    #[test]
    fn cassette_play_stop_commands_drive_transport() {
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        bus.scheduler.cancel(Event60::Vrtc);
        bus.scheduler.cancel(Event60::KeyScan);
        bus.scheduler.cancel(Event60::Scanline);
        bus.insert_cassette("p6", &[0x10, 0x20])
            .expect("tape parses");

        bus.io_write(0x90, SUB_COMMAND_CASSETTE_PLAY);
        assert_eq!(fire_next_event(&mut bus), Some(CASSETTE_DATA_VECTOR));
        assert_eq!(bus.sub.current_keycode(), 0x10);

        bus.io_write(0x90, SUB_COMMAND_CASSETTE_STOP);
        assert!(!bus.cassette_active);
        assert_eq!(bus.scheduler.next_event_cycle(), None);

        bus.io_write(0x90, SUB_COMMAND_CASSETTE_PLAY);
        assert_eq!(fire_next_event(&mut bus), Some(CASSETTE_DATA_VECTOR));
        assert_eq!(bus.sub.current_keycode(), 0x20);
    }

    #[test]
    fn key_scan_is_suppressed_while_the_tape_plays() {
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        bus.scheduler.cancel(Event60::Vrtc);
        bus.insert_cassette("p6", &[0x55]).expect("tape parses");
        bus.system_latch_write(MOTOR_ON_TIMER_OFF);
        // Isolate the key-scan path from cassette delivery.
        bus.scheduler.cancel(Event60::CassetteByte);

        bus.push_keyboard_scancode(0x41);
        assert_eq!(fire_next_event(&mut bus), None);
        assert_eq!(bus.sub.current_keycode(), 0x00);
    }

    #[test]
    fn stopping_the_motor_cancels_delivery() {
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        bus.scheduler.cancel(Event60::Vrtc);
        bus.scheduler.cancel(Event60::KeyScan);
        bus.scheduler.cancel(Event60::Scanline);
        bus.insert_cassette("p6", &[0x10, 0x20])
            .expect("tape parses");

        bus.system_latch_write(MOTOR_ON_TIMER_OFF);
        assert!(bus.scheduler.next_event_cycle().is_some());

        bus.system_latch_write(0x01);
        assert!(!bus.cassette_active);
        assert_eq!(bus.scheduler.next_event_cycle(), None);
    }

    #[test]
    fn m1_fetch_always_costs_one_wait_state() {
        use common::Bus;
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        let mut view = MainBusView { bus: &mut bus };
        // The wait applies regardless of the fetch address (low or high).
        let _ = view.fetch_opcode_byte(0x0000);
        assert_eq!(view.drain_wait_cycles(), 1);
        let _ = view.fetch_opcode_byte(0xC000);
        assert_eq!(view.drain_wait_cycles(), 1);
    }

    #[test]
    fn memory_access_below_0x8000_costs_one_wait_state() {
        use common::Bus;
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        let mut view = MainBusView { bus: &mut bus };
        let _ = view.read_byte(0x1234);
        assert_eq!(view.drain_wait_cycles(), 1);
        view.write_byte(0x4000, 0x00);
        assert_eq!(view.drain_wait_cycles(), 1);
        // The upper half is wait-free.
        let _ = view.read_byte(0x8000);
        view.write_byte(0xC000, 0x00);
        assert_eq!(view.drain_wait_cycles(), 0);
    }

    #[test]
    fn psg_io_block_costs_one_wait_state() {
        use common::Bus;
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        let mut view = MainBusView { bus: &mut bus };
        view.io_write_byte(0xA0, 0x00);
        assert_eq!(view.drain_wait_cycles(), 1);
        let _ = view.io_read_byte(0xA2);
        assert_eq!(view.drain_wait_cycles(), 1);
        // Other I/O ports are wait-free.
        let _ = view.io_read_byte(0xC1);
        assert_eq!(view.drain_wait_cycles(), 0);
    }

    #[test]
    fn traces_use_live_io_decode_and_interrupt_source() {
        use common::Bus;

        let mut bus =
            Pc6000Bus::new_with_trace_sink(Pc6000Model::Pc6001, 48_000, RecordingTrace::default());
        {
            let mut view = MainBusView { bus: &mut bus };
            Bus::io_read_byte(&mut view, 0x90);
            Bus::io_read_byte(&mut view, 0xC1);
        }
        bus.interrupt.raise(IrqSource::Joystick);
        {
            let mut view = MainBusView { bus: &mut bus };
            Bus::acknowledge_irq(&mut view);
        }

        assert!(bus.tracer().accesses[0].handled);
        assert!(!bus.tracer().accesses[1].handled);
        assert_eq!(
            bus.tracer().interrupts[0].line,
            Some(IrqSource::Joystick as u16)
        );
    }

    #[test]
    fn memory_trace_addresses_apply_the_z80_address_mask() {
        use common::Bus;

        let mut bus =
            Pc6000Bus::new_with_trace_sink(Pc6000Model::Pc6001, 48_000, RecordingTrace::default());
        {
            let mut view = MainBusView { bus: &mut bus };
            Bus::read_byte(&mut view, 0x1_1234);
            Bus::write_byte(&mut view, 0x1_5678, 0xA5);
            Bus::fetch_opcode_byte(&mut view, 0x1_9ABC);
        }

        assert_eq!(bus.tracer().accesses[0].address, 0x1234);
        assert_eq!(bus.tracer().accesses[1].address, 0x5678);
        assert_eq!(bus.tracer().accesses[2].address, 0x9ABC);
    }

    #[test]
    fn active_scanline_asserts_busreq_until_crt_is_blanked() {
        let mut bus: Pc6000Bus = Pc6000Bus::new(Pc6000Model::Pc6001, 48_000);
        // Firing the first scanline grabs the bus for the display.
        bus.set_current_cycle(bus.line_period);
        bus.process_events();
        assert!(bus.cpu_stalled());

        // Blanking the CRT (PPI port C bit 1 reset) releases the bus at once.
        bus.io_write(0x93, 0x02);
        assert!(!bus.cpu_stalled());

        // A subsequent scanline does not re-assert the stall while blanked.
        let next = bus.current_cycle + bus.line_period;
        bus.set_current_cycle(next);
        bus.process_events();
        assert!(!bus.cpu_stalled());
    }
}
