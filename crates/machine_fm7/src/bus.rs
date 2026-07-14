//! FM-7 system bus.

mod cassette;
mod fdc;
mod kanji;
mod keyboard;
mod main_io;
mod sound;
mod sub_io;
mod video;

use common::{
    BeeperKind, HostDateTimeProvider, NoTrace, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceContext, TraceEvent, TraceInterruptAction, TracePresentation,
    TraceSink, trace_id,
};
use device::{
    ay8910::Ay8910,
    beeper::Beeper,
    cassette::CassetteDeck,
    mb61vh010_alu::{AluMemory, Mb61vh010Alu},
    mb8877_fdc::Mb8877Fdc,
    mouse_fm7::MouseFm7,
    soundboard_fm7::Fm7Opn,
};
use software_renderer::{Fm7Renderer, RenderInputsFm7};

use self::{
    kanji::KanjiRom,
    keyboard::{
        Fm7Keyboard, KeycodeTableSet,
        encoder::{KeyboardEncoder, ScancodeMode},
    },
    video::{SubMemory, VideoState},
};
use crate::{
    config::{
        BEEPER_FREQUENCY_HZ, BEEPER_TICK_CLOCK_HZ, BootMode, ClockConfig, Fm7Model, PSG_CLOCK_HZ,
    },
    interrupt::MainInterrupts,
    memory::Fm7Memory,
    rom::LoadedRoms,
    scheduler::{EventFm7, Fm7Scheduler},
};

/// Width of the base FM-7 software-rendered surface.
const DISPLAY_WIDTH: u32 = 640;
/// Height of the base FM-7 software-rendered surface.
const DISPLAY_HEIGHT: u32 = 200;

/// First address of the sub CPU VRAM region (three planes, 16 KiB each).
const SUB_VRAM_START: u16 = 0x0000;
/// Last address of the sub CPU VRAM region.
const SUB_VRAM_END: u16 = 0xBFFF;
/// Shift selecting the VRAM plane from a sub CPU VRAM address.
const VRAM_PLANE_SHIFT: u16 = 14;
/// Mask selecting the VRAM plane index after shifting.
const VRAM_PLANE_MASK: u16 = 0x03;

/// Frame-rate numerator for the 59.94 Hz refresh.
const FRAME_RATE_NUMERATOR: u64 = 5994;
/// Frame-rate denominator for the 59.94 Hz refresh.
const FRAME_RATE_DENOMINATOR: u64 = 100;
/// Visible scanlines latched per frame.
const VISIBLE_SCANLINES: u16 = 200;
/// Total scanlines per frame, including vertical blanking. The per-line period
/// is derived from the exact 59.94 Hz frame period, approximating the measured
/// 63.5 us horizontal interval by about 0.3 percent.
const TOTAL_SCANLINES: u64 = 262;
/// Scanline on which the vertical sync pulse begins, one line after the last
/// visible line has been displayed.
const VSYNC_PULSE_START_LINE: u64 = 201;
/// Scanline on which the vertical sync pulse ends. The pulse lasts roughly
/// half a millisecond, or eight scanlines.
const VSYNC_PULSE_END_LINE: u64 = 209;
/// Displayed portion of one visible scanline, in tenths of a microsecond.
const HORIZONTAL_ACTIVE_TENTH_MICROS: u64 = 395;
/// Full scanline period, in tenths of a microsecond.
const HORIZONTAL_TOTAL_TENTH_MICROS: u64 = 635;

/// First address in the main CPU memory-mapped I/O page.
const MAIN_IO_START: u16 = 0xFD00;
/// Last address in the main CPU memory-mapped I/O page.
const MAIN_IO_END: u16 = 0xFDFF;
/// Low byte mask for ports in the main CPU I/O page.
const MAIN_IO_PORT_MASK: u16 = 0x00FF;
/// First address in the writable vector window.
const VECTOR_WAIT_START: u16 = 0xFFE0;
/// Last address in the forced reset-vector window.
const VECTOR_WAIT_END: u16 = 0xFFFF;

/// Timer IRQ period numerator in main-clock cycles.
const TIMER_IRQ_NUMERATOR: u64 = 10_000;
/// Timer IRQ period denominator in Hz.
const TIMER_IRQ_DENOMINATOR: u64 = 4_915_200;

/// Dummy acknowledge byte returned to the 6809 IRQ cycle.
const IRQ_ACKNOWLEDGE_BYTE: u8 = 0x00;

/// Open-bus value returned by inaccessible or unmapped reads.
const OPEN_BUS: u8 = 0xFF;

/// First address of the main-side shared-RAM window.
const SHARED_WINDOW_START: u16 = 0xFC80;
/// Last address of the main-side shared-RAM window.
const SHARED_WINDOW_END: u16 = 0xFCFF;
/// Index mask selecting one of the 128 shared-RAM bytes.
const SHARED_WINDOW_INDEX_MASK: u16 = 0x007F;

/// First address of the sub memory-mapped I/O region.
const SUB_IO_START: u16 = 0xD400;
/// Last address of the sub memory-mapped I/O region.
const SUB_IO_END: u16 = 0xD7FF;
/// `0xD430` write bit 7 masking the periodic sub display NMI (active low).
const SUB_MISC_NMI_MASK_BIT: u8 = 0x80;
/// `0xD430` write bit 6 selecting the displayed VRAM page.
const SUB_MISC_DISPLAY_PAGE_BIT: u8 = 0x40;
/// `0xD430` write bit 5 selecting the CPU/sub draw page.
const SUB_MISC_DRAW_PAGE_BIT: u8 = 0x20;
/// `0xD430` write bit 2 enabling fine display scroll.
const SUB_MISC_FINE_OFFSET_BIT: u8 = 0x04;
/// `0xD430` write bits 1-0 selecting the CG ROM window bank at `0xD800-0xDFFF`.
const SUB_MISC_CG_BANK_MASK: u8 = 0x03;
/// `0xD430` read base value: every bit floats high and the status bits below are
/// cleared when their condition is false.
const SUB_MISC_READ_BASE: u8 = 0xFF;
/// `0xD430` read bit 7 reporting the display is inside horizontal or vertical
/// blank; cleared while the beam is in the displayed portion.
const SUB_MISC_BLANK_BIT: u8 = 0x80;
/// `0xD430` read bit 4 reporting the ALU is idle; cleared while a hardware line
/// draw is in flight.
const SUB_MISC_ALU_IDLE_BIT: u8 = 0x10;
/// `0xD430` read bit 2 reporting the vertical sync pulse; cleared outside it.
const SUB_MISC_VSYNC_BIT: u8 = 0x04;

/// Period of the periodic display NMI delivered to the sub CPU, in microseconds.
const SUB_DISPLAY_NMI_PERIOD_MICROS: u64 = 20_000;
/// Period of the keyboard latch tick draining the keycode FIFO, in microseconds.
const KEYBOARD_LATCH_PERIOD_MICROS: u64 = 20_000;
/// Delay before the FM-77AV encoder re-raises ACK after accepting a byte, in
/// microseconds.
const ENCODER_ACK_DELAY_MICROS: u64 = 5;
/// Period of the FM-77AV encoder RTC one-second tick, in microseconds.
const RTC_SECOND_PERIOD_MICROS: u64 = 1_000_000;
/// Microseconds per millisecond, converting repeat timings to the delay helper.
const MICROS_PER_MILLI: u64 = 1_000;
/// Auto-disarm delay of a pending CLR read-modify-write, in microseconds (fast).
const BUSY_CLEAR_DISARM_MICROS_FAST: u64 = 2;
/// Auto-disarm delay of a pending CLR read-modify-write, in microseconds (slow).
const BUSY_CLEAR_DISARM_MICROS_SLOW: u64 = 4;
/// Delay before the busy flag is re-set after a CLR write, in microseconds (fast).
const BUSY_DELAYED_SET_MICROS_FAST: u64 = 3;
/// Delay before the busy flag is re-set after a CLR write, in microseconds (slow).
const BUSY_DELAYED_SET_MICROS_SLOW: u64 = 6;

/// Divisor applied to the sub clock while it contends with the main CPU for VRAM.
const VRAM_CONTENTION_DIVISOR: u32 = 3;

/// Main MMIO accesses per inserted wait cycle in the plain address mode.
const IO_WAIT_PERIOD_ACCESSES: u8 = 2;
/// Main MMIO accesses per inserted wait cycle during the long phase while the
/// MMR or the relocatable window translates addresses. The hardware alternates
/// a three-access and a two-access period, averaging one wait per 2.5 accesses.
const IO_WAIT_PERIOD_MMR_LONG_ACCESSES: u8 = 3;

/// FM-7 / FM-77AV system bus.
pub struct Fm7Bus<T: TraceSink = NoTrace> {
    model: Fm7Model,
    clocks: ClockConfig,
    boot_mode: BootMode,
    memory: Fm7Memory,
    interrupts: MainInterrupts,
    keyboard: Fm7Keyboard,
    /// FM-77AV serial keyboard encoder and its embedded RTC; inert on the FM-7.
    encoder: KeyboardEncoder,
    /// Event scheduler in main-clock cycles.
    pub(crate) scheduler: Fm7Scheduler,
    current_cycle: u64,
    wait_cycles: i64,
    clock_fast: bool,
    /// Set when the effective main clock changes, so the machine re-anchors the
    /// sub CPU cycle accumulator on the next slice.
    clock_reanchor_pending: bool,
    /// Main MMIO accesses accumulated toward the next inserted wait cycle.
    io_wait_counter: u8,
    /// Set while the wait period is in its three-access phase (MMR mode only).
    io_wait_long_phase: bool,
    sub_memory: SubMemory,
    sub_cycle: u64,
    sub_clock_credit: u64,
    sub_clock_hz: u32,
    cycle_steal_enabled: bool,
    vram_access_flag: bool,
    sub_halt_requested: bool,
    sub_halted: bool,
    sub_busy: bool,
    busy_clear_pending: bool,
    cancel_request: bool,
    sub_nmi_pending: bool,
    /// Whether the periodic sub display NMI is masked (FM-77AV `0xD430` bit 7).
    /// The FM-7 delivers the NMI unconditionally, so it stays clear there.
    sub_nmi_masked: bool,
    sub_beep_requested: bool,
    /// Set when `0xFD13` switches the sub-monitor bank, so the machine pulse-resets
    /// the sub CPU on the next slice.
    sub_reset_pending: bool,
    /// AY-3-8910 PSG driven through the `0xFD0D`/`0xFD0E` command latch (FM-7).
    psg: Ay8910,
    /// Current PSG latch command: 0 inactive, 1 read, 2 write data, 3 latch
    /// address (masked to two bits like the hardware command register).
    psg_command: u8,
    /// Last byte written to the PSG data port `0xFD0E`, acted on by the command.
    psg_data_latch: u8,
    /// YM2203 (OPN) replacing the PSG on the FM-77AV; `None` on the FM-7.
    opn: Option<Fm7Opn>,
    /// Current OPN latch command driving the `0xFD0D`/`0xFD0E` and `0xFD15`/`0xFD16`
    /// port protocol: 0 inactive, 1 read, 2 write data, 3 latch address, 4 read
    /// status, 9 read joystick.
    opn_command: u8,
    /// Last byte written to the OPN data port, acted on by the current command.
    opn_data_latch: u8,
    /// Mirror of the OPN register address latch, used to decode SSG port B
    /// writes for the mouse strobe.
    opn_address_latch: u8,
    /// Last value written to SSG port B: joystick column select, mouse strobe,
    /// and button gate lines.
    opn_port_b: u8,
    /// Joystick-port mouse read through the SSG parallel ports.
    mouse: MouseFm7,
    /// Whether the mouse (instead of the joystick) currently owns the shared
    /// port, decided by whichever device the host used last.
    mouse_selected: bool,
    /// Whether the mouse interrupt path is enabled (`0xFD17` bit 2).
    opn_mouse_enabled: bool,
    /// Cycle at which the current OPN audio output frame started.
    audio_frame_start_cycle: u64,
    /// Fixed 1200 Hz buzzer gated through `0xFD03` and the sub CPU beep request.
    beeper: Beeper,
    /// Whether the continuous buzzer gate (`0xFD03` bit 7) is held.
    beeper_continuous_gate: bool,
    /// Whether a one-shot buzzer pulse (`0xFD03` bit 6 or the sub request) is
    /// currently armed.
    beeper_one_shot_active: bool,
    /// Joystick pad state encoded for PSG parallel port A, active low.
    joystick_port_a: u8,
    /// MB8877 floppy disk controller at `0xFD18-0xFD1F`.
    fdc: Mb8877Fdc,
    /// Selected head/side latched by `0xFD1C` (only bit 0 is meaningful).
    fdc_side: u8,
    /// Selected drive index latched by `0xFD1D` (two bits).
    fdc_drive_select: u8,
    /// Whether the controller currently sees the motor spinning (post spin-up).
    fdc_motor_on: bool,
    /// Last motor state requested by `0xFD1D` bit 7, tracked to detect changes.
    fdc_motor_requested: bool,
    /// Cassette data recorder gated by `0xFD00` and sampled through `0xFD02`.
    cassette: CassetteDeck,
    /// JIS level-1 kanji ROM addressed through `0xFD20-0xFD23`.
    kanji: KanjiRom,
    video: VideoState,
    /// FM-77AV MB61VH010 graphics ALU; inert on the FM-7 (never enabled).
    alu: Mb61vh010Alu,
    renderer: Fm7Renderer,
    display_width: u32,
    display_height: u32,
    frame_number: u64,
    scanline: u16,
    /// Cycle at which the current frame began, anchoring beam-position reads.
    frame_start_cycle: u64,
    tracer: T,
}

impl Fm7Bus<NoTrace> {
    /// Creates an untraced bus for `model`, booting according to `boot_mode`.
    pub fn new(model: Fm7Model, boot_mode: BootMode, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(model, boot_mode, sample_rate, NoTrace)
    }
}

/// A borrowed view onto the sub CPU VRAM planes for the graphics ALU. It
/// resolves an in-plane byte offset and plane index to physical storage through
/// the display page/scroll decode and enforces the `0xFD37` per-plane access
/// mask so a blocked plane reads open bus and drops writes.
struct AluVramView<'a> {
    video: &'a VideoState,
    sub_memory: &'a mut SubMemory,
}

impl AluMemory for AluVramView<'_> {
    fn read_plane(&self, offset: u16, plane: u8) -> u8 {
        if !self.video.vram_read_allowed(plane) {
            return OPEN_BUS;
        }
        let index = self
            .video
            .translate_vram_address((u16::from(plane) << VRAM_PLANE_SHIFT) | offset);
        self.sub_memory.vram_byte(index)
    }

    fn write_plane(&mut self, offset: u16, plane: u8, value: u8) {
        if !self.video.vram_write_allowed(plane) {
            return;
        }
        let index = self
            .video
            .translate_vram_address((u16::from(plane) << VRAM_PLANE_SHIFT) | offset);
        self.sub_memory.set_vram_byte(index, value);
    }

    fn pixel_width(&self) -> u32 {
        self.video.pixel_width()
    }
}

impl<T: TraceSink> Fm7Bus<T> {
    /// Creates a traced bus for `model`, booting according to `boot_mode`.
    pub fn new_with_trace_sink(
        model: Fm7Model,
        boot_mode: BootMode,
        sample_rate: u32,
        tracer: T,
    ) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sample_rate,
        };
        let mut bus = Self {
            model,
            clocks,
            boot_mode,
            memory: Fm7Memory::empty(model, boot_mode),
            interrupts: MainInterrupts::new(),
            keyboard: Fm7Keyboard::new(),
            encoder: KeyboardEncoder::new(),
            scheduler: Fm7Scheduler::new(),
            current_cycle: 0,
            wait_cycles: 0,
            clock_fast: true,
            clock_reanchor_pending: false,
            io_wait_counter: 0,
            io_wait_long_phase: false,
            sub_memory: SubMemory::new(),
            sub_cycle: 0,
            sub_clock_credit: 0,
            sub_clock_hz: model.sub_clock_hz(),
            cycle_steal_enabled: model.cycle_steal_default(),
            vram_access_flag: false,
            sub_halt_requested: false,
            sub_halted: false,
            sub_busy: false,
            busy_clear_pending: false,
            cancel_request: false,
            sub_nmi_pending: false,
            sub_nmi_masked: false,
            sub_beep_requested: false,
            sub_reset_pending: false,
            psg: Ay8910::new(),
            psg_command: 0,
            psg_data_latch: 0,
            opn: if model.has_opn() {
                Some(Fm7Opn::new(model.main_clock_hz(), sample_rate))
            } else {
                None
            },
            opn_command: 0,
            opn_data_latch: 0,
            opn_address_latch: 0,
            opn_port_b: 0,
            mouse: MouseFm7::new(),
            mouse_selected: true,
            opn_mouse_enabled: false,
            audio_frame_start_cycle: 0,
            beeper: Beeper::new(
                BeeperKind::Fixed {
                    hz: BEEPER_FREQUENCY_HZ,
                },
                BEEPER_TICK_CLOCK_HZ,
            ),
            beeper_continuous_gate: false,
            beeper_one_shot_active: false,
            joystick_port_a: sound::JOYSTICK_IDLE,
            fdc: fdc::new_fdc(model.main_clock_hz()),
            fdc_side: 0,
            fdc_drive_select: 0,
            fdc_motor_on: false,
            fdc_motor_requested: false,
            cassette: CassetteDeck::new(),
            kanji: KanjiRom::new(),
            video: VideoState::new(),
            alu: Mb61vh010Alu::new(),
            renderer: Fm7Renderer::new(),
            display_width: DISPLAY_WIDTH,
            display_height: DISPLAY_HEIGHT,
            frame_number: 0,
            scanline: 0,
            frame_start_cycle: 0,
            tracer,
        };
        bus.schedule_timer();
        bus.schedule_sub_display_nmi();
        bus.schedule_keyboard_latch();
        bus.schedule_vblank();
        bus.schedule_scanline();
        if model.has_mmr() {
            bus.schedule_rtc_second();
        }
        bus
    }

    /// The machine model this bus is configured for.
    pub fn model(&self) -> Fm7Model {
        self.model
    }

    /// The configured boot mode.
    pub fn boot_mode(&self) -> BootMode {
        self.boot_mode
    }

    /// The clock configuration.
    pub fn clocks(&self) -> ClockConfig {
        self.clocks
    }

    /// Main CPU clock in Hz.
    ///
    /// On the FM-77AV the fast clock drops from 1.798 MHz to the MMR rate while
    /// MMR translation or the relocatable window is enabled; the FM-7 has no MMR
    /// and always uses its normal fast clock.
    pub fn cpu_clock_hz(&self) -> u32 {
        if self.clock_fast {
            if self.model.has_mmr() && self.memory.mmr_translation_active() {
                self.model.main_clock_mmr_hz()
            } else {
                self.clocks.main_clock_hz
            }
        } else {
            self.model.main_clock_slow_hz()
        }
    }

    /// The current monotonic cycle count.
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Advances the monotonic cycle counter.
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    /// The cycle of the next scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Immutable access to the tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Mutable access to the tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Installs the loaded ROM set into the main and sub memory maps.
    pub fn load_roms(&mut self, roms: &LoadedRoms) {
        debug_assert_eq!(roms.model, self.model);
        self.memory = Fm7Memory::new(roms, self.boot_mode);
        self.sub_memory.install_roms(roms);
        self.kanji.install_rom(roms.kanji.as_deref());
    }

    /// Reads a main CPU byte and reports whether its address was decoded.
    ///
    /// The shared-RAM window is only readable while the sub CPU is halted;
    /// otherwise it floats to open bus.
    pub fn read_byte(&mut self, address: u16) -> (u8, bool) {
        self.charge_main_access_wait(address);
        match address {
            MAIN_IO_START..=MAIN_IO_END => self.main_io_read((address & MAIN_IO_PORT_MASK) as u8),
            SHARED_WINDOW_START..=SHARED_WINDOW_END => {
                let value = if self.sub_halted {
                    self.sub_memory
                        .shared_ram_byte((address & SHARED_WINDOW_INDEX_MASK) as u8)
                } else {
                    OPEN_BUS
                };
                (value, true)
            }
            _ => {
                let value = match self.memory.direct_vram_target(address) {
                    Some(sub_address) if self.sub_halted => self.read_sub_space_byte(sub_address).0,
                    Some(_) => OPEN_BUS,
                    None => self.memory.read(address),
                };
                (value, true)
            }
        }
    }

    /// Writes a main CPU byte and reports whether its address was decoded.
    ///
    /// The shared-RAM window is only writable while the sub CPU is halted;
    /// otherwise the write is dropped.
    pub fn write_byte(&mut self, address: u16, value: u8) -> bool {
        self.charge_main_access_wait(address);
        match address {
            MAIN_IO_START..=MAIN_IO_END => {
                self.main_io_write((address & MAIN_IO_PORT_MASK) as u8, value)
            }
            SHARED_WINDOW_START..=SHARED_WINDOW_END => {
                if self.sub_halted {
                    self.sub_memory
                        .set_shared_ram_byte((address & SHARED_WINDOW_INDEX_MASK) as u8, value);
                }
                true
            }
            _ => match self.memory.direct_vram_target(address) {
                Some(sub_address) if self.sub_halted => {
                    self.write_sub_space_byte(sub_address, value);
                    true
                }
                Some(_) => true,
                None => {
                    self.memory.write(address, value);
                    true
                }
            },
        }
    }

    /// Reads a byte through the memory map for tests and tooling.
    pub fn peek_byte(&self, address: u16) -> u8 {
        self.memory.read(address)
    }

    /// Writes a byte through the memory map for tests and tooling.
    pub fn poke_byte(&mut self, address: u16, value: u8) {
        self.memory.write(address, value);
    }

    /// Whether the FM-77AV ALU is present and enabled, so it intercepts VRAM
    /// accesses in place of the plain store.
    fn alu_enabled(&self) -> bool {
        self.model().has_alu() && self.alu.is_enabled()
    }

    /// Runs the enabled ALU over the sub CPU VRAM byte at `address`, driving the
    /// ALU with a view onto the VRAM planes and the multipage access mask.
    fn alu_access_vram(&mut self, address: u16) {
        let Self {
            alu,
            video,
            sub_memory,
            ..
        } = self;
        let mut memory = AluVramView { video, sub_memory };
        alu.access_vram(&mut memory, address);
    }

    /// Writes an ALU register on behalf of the sub I/O decode, arming the busy
    /// timer when the write triggered a hardware line draw.
    fn alu_register_write(&mut self, port: u8, value: u8) {
        let busy_micros = {
            let Self {
                alu,
                video,
                sub_memory,
                ..
            } = self;
            let mut memory = AluVramView { video, sub_memory };
            alu.write_register(&mut memory, port, value)
        };
        if busy_micros > 0 {
            self.schedule_alu_busy_clear(busy_micros);
        }
    }

    /// Reads the sub address space and reports whether it was decoded.
    pub(crate) fn read_sub_space_byte(&mut self, address: u16) -> (u8, bool) {
        match address {
            SUB_IO_START..=SUB_IO_END => {
                if self.sub_memory.hidden_ram_mapped(address) {
                    (self.sub_memory.read(address), true)
                } else {
                    self.sub_io_read((address & 0x00FF) as u8)
                }
            }
            SUB_VRAM_START..=SUB_VRAM_END => {
                if self.alu_enabled() {
                    self.alu_access_vram(address);
                }
                let plane = ((address >> VRAM_PLANE_SHIFT) & VRAM_PLANE_MASK) as u8;
                if self.video.vram_read_allowed(plane) {
                    let index = self.video.translate_vram_address(address);
                    (self.sub_memory.vram_byte(index), true)
                } else {
                    (OPEN_BUS, true)
                }
            }
            _ => (self.sub_memory.read(address), true),
        }
    }

    /// Writes the sub address space and reports whether it was decoded.
    pub(crate) fn write_sub_space_byte(&mut self, address: u16, value: u8) -> bool {
        match address {
            SUB_IO_START..=SUB_IO_END => {
                if self.sub_memory.hidden_ram_mapped(address) {
                    self.sub_memory.write(address, value);
                    true
                } else {
                    self.sub_io_write((address & 0x00FF) as u8, value)
                }
            }
            SUB_VRAM_START..=SUB_VRAM_END => {
                if self.alu_enabled() {
                    self.alu_access_vram(address);
                    return true;
                }
                let plane = ((address >> VRAM_PLANE_SHIFT) & VRAM_PLANE_MASK) as u8;
                if self.video.vram_write_allowed(plane) {
                    let index = self.video.translate_vram_address(address);
                    self.sub_memory.set_vram_byte(index, value);
                }
                true
            }
            _ => {
                self.sub_memory.write(address, value);
                true
            }
        }
    }

    /// Whether the F-BASIC ROM bank is mapped at `0x8000-0xFBFF`.
    pub fn basic_rom_mapped(&self) -> bool {
        self.memory.basic_rom_mapped()
    }

    /// Whether the FM-77AV initiator ROM overlay is currently active.
    pub fn initiator_enabled(&self) -> bool {
        self.memory.initiator_enabled()
    }

    /// Processes all scheduler events due at the current cycle.
    pub fn process_events(&mut self) {
        self.poll_sub_beep_request();
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
                EventFm7::TimerIrq => {
                    self.interrupts.set_timer_pending(
                        true,
                        TraceContext::main_cpu(
                            self.current_cycle,
                            Some(u64::from(self.cpu_clock_hz())),
                        ),
                        &mut self.tracer,
                    );
                    self.schedule_timer();
                }
                EventFm7::SubDisplayNmi => {
                    // The hardware does not deliver the periodic display NMI while
                    // the sub CPU is halted by the main CPU; the pulse is simply
                    // dropped rather than latched (a >40 ms halt would otherwise
                    // double-fire it).
                    if !self.sub_nmi_masked && !self.sub_halted {
                        self.sub_nmi_pending = true;
                    }
                    self.schedule_sub_display_nmi();
                }
                EventFm7::SubBusyClearDelay => {
                    self.sub_busy = true;
                }
                EventFm7::SubBusyDelayDisarm => {
                    self.busy_clear_pending = false;
                }
                EventFm7::KeyboardLatch => {
                    if self.keyboard.latch_next() {
                        self.interrupts.set_keyboard_pending(
                            true,
                            TraceContext::main_cpu(
                                self.current_cycle,
                                Some(u64::from(self.cpu_clock_hz())),
                            ),
                            &mut self.tracer,
                        );
                    }
                    self.schedule_keyboard_latch();
                }
                EventFm7::VBlank => {
                    self.present_latched_frame();
                    self.trace_presentation();
                    self.frame_number = self.frame_number.wrapping_add(1);
                    self.frame_start_cycle = event.fire_cycle;
                    self.video.commit_frame_palette();
                    self.video.commit_frame_analog_palette();
                    self.renderer.clear_latched_frame();
                    self.scanline = 0;
                    self.schedule_vblank();
                    self.schedule_scanline();
                }
                EventFm7::Scanline => {
                    self.advance_cassette();
                    if self.scanline < VISIBLE_SCANLINES {
                        self.latch_scanline(self.scanline);
                        self.scanline += 1;
                        if self.scanline < VISIBLE_SCANLINES {
                            self.schedule_scanline();
                        }
                    }
                }
                EventFm7::BeepOneShotOff => self.end_beep_one_shot(),
                EventFm7::FdcMotorOn => self.on_fdc_motor_on(),
                EventFm7::FdcMotorOff => self.on_fdc_motor_off(),
                EventFm7::FdcSeekComplete => self.on_fdc_seek_complete(self.current_cycle),
                EventFm7::OpnTimerA => self.on_opn_timer(0),
                EventFm7::OpnTimerB => self.on_opn_timer(1),
                EventFm7::AluBusyClear => self.clear_alu_busy(),
                EventFm7::EncoderAck => self.encoder.acknowledge(),
                EventFm7::KeyboardRepeat => self.on_keyboard_repeat(),
                EventFm7::RtcSecond => {
                    self.encoder.advance_one_second();
                    self.schedule_rtc_second();
                }
                EventFm7::MouseTimeout => self.on_mouse_timeout(),
            }
        }
    }

    /// Whether an IRQ is pending for the main CPU.
    pub fn has_irq(&self) -> bool {
        self.interrupts.irq_line()
    }

    /// Acknowledges the main CPU IRQ line.
    pub fn acknowledge_irq(&mut self) -> u8 {
        IRQ_ACKNOWLEDGE_BYTE
    }

    /// Whether the main CPU FIRQ line is active.
    pub fn firq_active(&self) -> bool {
        self.interrupts.firq_line()
    }

    /// Injects a keyboard event. `code` carries the FM-7 physical scancode in its
    /// low seven bits with bit 7 set on release. The keycode reaches the read
    /// latch on the next keyboard latch tick; the BREAK key updates the main FIRQ
    /// immediately because it bypasses the keycode path. The FM-77AV encoder
    /// selects the reporting mode: raw scan mode enqueues physical make/break
    /// scancodes, the translated modes enqueue keycodes from their table set.
    pub fn push_keyboard_scancode(&mut self, code: u8) {
        let pressed = code & 0x80 == 0;
        let scancode = code & 0x7F;
        if self.model.has_mmr() && self.encoder.scancode_mode() == ScancodeMode::Scan {
            self.keyboard.push_scan(scancode, pressed);
        } else {
            self.keyboard
                .push(scancode, pressed, self.keycode_table_set());
        }
        self.interrupts.set_break(
            self.keyboard.break_pressed(),
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            &mut self.tracer,
        );
        if self.model.has_mmr() {
            self.update_auto_repeat(scancode, pressed);
        }
    }

    /// Whether the sub CPU keyboard FIRQ line is asserted. The sub CPU handles the
    /// keyboard by default, so the FIRQ is the inverse of the main keyboard IRQ
    /// mask: it is delivered while the main CPU has not claimed the keyboard IRQ
    /// through `0xFD02` bit 0.
    pub fn sub_has_firq(&self) -> bool {
        self.keyboard.interrupt_asserted() && !self.interrupts.keyboard_irq_enabled()
    }

    /// Forces the timer pending latch for tests and simple device plumbing.
    pub fn set_timer_pending(&mut self, pending: bool) {
        self.interrupts.set_timer_pending(
            pending,
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            &mut self.tracer,
        );
    }

    /// Acknowledges the timer pending latch.
    pub fn ack_timer(&mut self) {
        self.interrupts.ack_timer(
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            &mut self.tracer,
        );
    }

    /// The timer IRQ period in active main-clock cycles.
    pub fn timer_irq_period_cycles(&self) -> u64 {
        (TIMER_IRQ_NUMERATOR * u64::from(self.cpu_clock_hz()) / TIMER_IRQ_DENOMINATOR).max(1)
    }

    /// The last rendered framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// The framebuffer dimensions.
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// The committed display start offset of the currently shown page.
    pub fn display_offset(&self) -> u16 {
        self.video.display_offset(self.video.display_page())
    }

    /// The main-clock cycles in one 59.94 Hz frame at the current CPU speed.
    pub fn frame_period_cycles(&self) -> u64 {
        (u64::from(self.cpu_clock_hz()) * FRAME_RATE_DENOMINATOR / FRAME_RATE_NUMERATOR).max(1)
    }

    /// The main-clock cycles between successive scanline latches.
    pub fn scanline_period_cycles(&self) -> u64 {
        (self.frame_period_cycles() / TOTAL_SCANLINES).max(1)
    }

    /// The scanline index the beam is on within the current frame.
    fn current_frame_line(&self) -> u64 {
        let position = self.current_cycle.saturating_sub(self.frame_start_cycle);
        position / self.scanline_period_cycles()
    }

    /// Whether the beam is inside the vertical sync pulse.
    pub(crate) fn vsync_active(&self) -> bool {
        (VSYNC_PULSE_START_LINE..VSYNC_PULSE_END_LINE).contains(&self.current_frame_line())
    }

    /// Whether the beam is inside the displayed portion of a visible scanline.
    pub(crate) fn display_active(&self) -> bool {
        let position = self.current_cycle.saturating_sub(self.frame_start_cycle);
        let period = self.scanline_period_cycles();
        if position / period >= u64::from(VISIBLE_SCANLINES) {
            return false;
        }
        let position_in_line = position % period;
        position_in_line * HORIZONTAL_TOTAL_TENTH_MICROS < period * HORIZONTAL_ACTIVE_TENTH_MICROS
    }

    /// Schedules the next frame-start (vertical blank) event.
    fn schedule_vblank(&mut self) {
        let period = self.frame_period_cycles();
        self.scheduler
            .schedule(EventFm7::VBlank, self.current_cycle + period);
    }

    /// Schedules the next scanline-latch event.
    fn schedule_scanline(&mut self) {
        let period = self.scanline_period_cycles();
        self.scheduler
            .schedule(EventFm7::Scanline, self.current_cycle + period);
    }

    /// Latches one visible scanline from the live VRAM and display registers,
    /// using the frame-latched palette snapshot.
    fn latch_scanline(&mut self, line: u16) {
        let display_page = self.video.display_page();
        let inputs = RenderInputsFm7 {
            planes: self.sub_memory.vram(),
            digital_palette: self.video.frame_digital_palette(),
            analog_palette: self.video.frame_analog_palette(),
            display_mask: self.video.display_mask(),
            display_offsets: [
                self.video.display_offset(false),
                self.video.display_offset(true),
            ],
            crt_enabled: self.video.crt_enabled(),
            mode320: self.video.mode320(),
            display_page,
        };
        self.renderer.latch_scanline(&inputs, usize::from(line));
    }

    /// Composites the latched scanlines and records the presented dimensions.
    fn present_latched_frame(&mut self) {
        let (width, height) = self.renderer.present_latched_frame();
        self.display_width = width;
        self.display_height = height;
    }

    fn trace_presentation(&mut self) {
        if !T::ENABLED {
            return;
        }
        self.tracer.trace(
            TraceContext::presentation_main(
                self.current_cycle,
                Some(u64::from(self.cpu_clock_hz())),
            ),
            TraceEvent::Presentation(TracePresentation {
                display: trace_id::display::MAIN,
                frame: self.frame_number.saturating_add(1),
                width: self.display_width,
                height: self.display_height,
            }),
        );
    }

    /// Generates audio samples into `output` for the main-clock cycles elapsed
    /// since the previous call and returns the number of interleaved stereo
    /// samples written.
    ///
    /// The PSG is the base writer: it overwrites the buffer and its returned
    /// count is authoritative for host speed pacing. The buzzer mixes additively
    /// on top without changing that count. Both devices track their own frame
    /// span, so they stay in step across calls.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        if self.opn.is_some() {
            return self.generate_opn_audio(volume, output);
        }

        let cpu_clock_hz = self.clocks.main_clock_hz;
        let sample_rate = self.clocks.sample_rate;
        let frame_end = self.current_cycle;

        let count = self.psg.generate_samples(
            frame_end,
            PSG_CLOCK_HZ,
            cpu_clock_hz,
            sample_rate,
            volume,
            output,
        );
        self.beeper.mix_samples(
            frame_end,
            cpu_clock_hz,
            BEEPER_TICK_CLOCK_HZ,
            sample_rate,
            volume,
            output,
        );
        count
    }

    /// Returns the font ROM data exposed to the image selector.
    pub fn font_rom_data(&self) -> &[u8] {
        &[]
    }

    /// Converts elapsed main-clock cycles into the sub CPU cycles it owes,
    /// carrying the fractional remainder so the rational clock ratio holds exactly.
    pub(crate) fn sub_cycles_for_main_units(&mut self, main_units: u64) -> u64 {
        let main_hz = u128::from(self.cpu_clock_hz().max(1));
        let total = u128::from(self.sub_clock_credit)
            + u128::from(main_units) * u128::from(self.sub_clock_hz);
        let sub_cycles = total / main_hz;
        self.sub_clock_credit = (total % main_hz) as u64;
        u64::try_from(sub_cycles).unwrap_or(u64::MAX)
    }

    /// Whether the main CPU has requested the sub CPU to halt.
    pub(crate) fn sub_halt_active(&self) -> bool {
        self.sub_halt_requested
    }

    /// Sets the sub halt-acknowledge state. Entering halt also raises the busy
    /// flag, mirroring the sub CPU asserting its bus-available lines.
    pub(crate) fn set_sub_halted(&mut self, halted: bool) {
        self.sub_halted = halted;
        if halted {
            self.sub_busy = true;
        }
    }

    /// Whether a handshake that benefits from tight interleaving is in progress.
    pub(crate) fn handshake_active(&self) -> bool {
        self.sub_halt_requested || self.cancel_request
    }

    /// Advances the sub CPU's private cycle timestamp while it is idle.
    pub(crate) fn advance_sub_cycle(&mut self, cycles: u64) {
        self.sub_cycle = self.sub_cycle.saturating_add(cycles);
    }

    /// Whether the sub CPU IRQ line (CANCEL) is asserted.
    pub(crate) fn sub_has_irq(&self) -> bool {
        self.cancel_request
    }

    /// Acknowledges the sub CPU IRQ vector cycle. The CANCEL line itself is
    /// cleared by the sub reading its acknowledge port, not by this cycle.
    pub(crate) fn acknowledge_sub_irq(&mut self) -> u8 {
        IRQ_ACKNOWLEDGE_BYTE
    }

    /// Whether the sub CPU NMI line (periodic display NMI) is asserted.
    pub(crate) fn sub_has_nmi(&self) -> bool {
        self.sub_nmi_pending
    }

    /// Clears the sub CPU NMI line after the core services it.
    pub(crate) fn acknowledge_sub_nmi(&mut self) {
        self.sub_nmi_pending = false;
    }

    /// Clears the pending CANCEL request after the sub acknowledges it.
    pub(crate) fn acknowledge_cancel(&mut self) {
        self.cancel_request = false;
    }

    /// Raises the CANCEL request, asserting the sub CPU IRQ line.
    pub(crate) fn raise_cancel_request(&mut self) {
        self.cancel_request = true;
    }

    /// Records that the sub CPU has requested a beeper one-shot. The pending flag
    /// is drained at the next event boundary, which fires the shared one-shot.
    pub(crate) fn request_sub_beep(&mut self) {
        self.sub_beep_requested = true;
    }

    /// Raises the main ATTENTION FIRQ on behalf of the sub CPU.
    pub(crate) fn raise_sub_attention(&mut self) {
        self.interrupts.raise_sub_attention(
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz()))),
            &mut self.tracer,
        );
    }

    /// Writes the FM-77AV sub misc register (`0xD430`). Bit 7 masks the periodic
    /// sub display NMI (active low: set = masked); masking also clears any NMI
    /// already latched. Bit 6 selects the displayed VRAM page, bit 5 the CPU/sub
    /// draw page, bit 2 enables fine display scroll, and bits 1-0 select the CG
    /// ROM window bank.
    pub(crate) fn write_sub_misc_register(&mut self, value: u8) {
        self.sub_nmi_masked = value & SUB_MISC_NMI_MASK_BIT != 0;
        if self.sub_nmi_masked {
            self.sub_nmi_pending = false;
        }
        self.video
            .set_display_page(value & SUB_MISC_DISPLAY_PAGE_BIT != 0);
        self.video
            .set_active_page(value & SUB_MISC_DRAW_PAGE_BIT != 0);
        self.video
            .set_fine_offset_enabled(value & SUB_MISC_FINE_OFFSET_BIT != 0);
        self.sub_memory
            .set_cg_window_bank(value & SUB_MISC_CG_BANK_MASK);
    }

    /// Reads the FM-77AV sub misc register (`0xD430`): the horizontal-blank,
    /// vertical-sync and ALU-idle status the sub monitor polls for display
    /// timing. The ALU-idle bit is set while no hardware line draw is in flight,
    /// and the display is reported outside horizontal blank.
    pub(crate) fn read_sub_misc_register(&self) -> u8 {
        let mut value = SUB_MISC_READ_BASE;
        if self.display_active() {
            value &= !SUB_MISC_BLANK_BIT;
        }
        if self.alu.is_busy() {
            value &= !SUB_MISC_ALU_IDLE_BIT;
        }
        if !self.vsync_active() {
            value &= !SUB_MISC_VSYNC_BIT;
        }
        value
    }

    /// Selects the FM-77AV sub-monitor bank (`0xFD13`) and requests the pulse
    /// reset that re-vectors the sub CPU into the newly banked monitor.
    pub(crate) fn set_sub_monitor_bank(&mut self, bank: u8) {
        self.sub_memory.set_sub_monitor_bank(bank);
        self.sub_reset_pending = true;
    }

    /// Returns and clears a pending sub-monitor bank-switch reset request.
    pub(crate) fn take_sub_reset(&mut self) -> bool {
        core::mem::take(&mut self.sub_reset_pending)
    }

    /// The active FM-77AV sub-monitor bank.
    pub fn sub_monitor_bank(&self) -> u8 {
        self.sub_memory.sub_monitor_bank()
    }

    /// Sets or clears the VRAM access flag and recomputes the sub clock rate.
    pub(crate) fn set_vram_access_flag(&mut self, active: bool) {
        self.vram_access_flag = active;
        self.update_sub_clock();
    }

    /// Enables or disables sub-CPU cycle steal (`0xD405`, FM-77AV). With cycle
    /// steal on, the sub CPU no longer runs at a third of its clock while it
    /// touches VRAM.
    pub(crate) fn set_cycle_steal(&mut self, enabled: bool) {
        self.cycle_steal_enabled = enabled;
        self.update_sub_clock();
    }

    /// Clears the busy flag on a sub read of the busy port and arms the pending
    /// clear-window used to detect a read-modify-write CLR sequence.
    pub(crate) fn clear_sub_busy_on_read(&mut self) {
        self.sub_busy = false;
        self.busy_clear_pending = true;
        let delay = self.busy_clear_disarm_cycles();
        self.scheduler
            .schedule(EventFm7::SubBusyDelayDisarm, self.current_cycle + delay);
    }

    /// Sets the busy flag on a sub write of the busy port. When the write closes a
    /// CLR read-modify-write, the busy flag stays cleared and is re-set only after
    /// a short delay.
    pub(crate) fn set_sub_busy_on_write(&mut self) {
        if self.busy_clear_pending {
            self.busy_clear_pending = false;
            self.sub_busy = false;
            self.scheduler.cancel(EventFm7::SubBusyDelayDisarm);
            let delay = self.busy_delayed_set_cycles();
            self.scheduler
                .schedule(EventFm7::SubBusyClearDelay, self.current_cycle + delay);
        } else {
            self.sub_busy = true;
        }
    }

    /// Whether the sub CPU is currently halt-acknowledged.
    pub fn is_sub_halted(&self) -> bool {
        self.sub_halted
    }

    /// The current sub busy flag state, mirrored to main `0xFD04`/`0xFD05` bit 7.
    pub fn sub_busy(&self) -> bool {
        self.sub_busy
    }

    /// The sub CPU's private cycle count.
    pub fn sub_cycle(&self) -> u64 {
        self.sub_cycle
    }

    /// Whether the sub CPU has requested a beeper one-shot since the last check.
    pub fn sub_beep_requested(&self) -> bool {
        self.sub_beep_requested
    }

    /// Reads a byte from the sub address space without side effects, for tests.
    pub fn sub_peek_byte(&self, address: u16) -> u8 {
        self.sub_memory.read(address)
    }

    /// Writes a byte anywhere in the sub address space, ROM included, for tests.
    pub fn sub_poke_byte(&mut self, address: u16, value: u8) {
        self.sub_memory.force_write(address, value);
    }

    /// Writes the FM-77AV MMR control register (`0xFD93`), re-anchoring the sub
    /// clock accumulator when the enable bits change the effective main clock.
    pub(crate) fn write_mmr_control(&mut self, value: u8) {
        let before = self.cpu_clock_hz();
        self.memory.write_mmr_control(value);
        if self.cpu_clock_hz() != before {
            self.note_main_clock_change();
        }
    }

    /// Records a main-clock change: the sub credit remainder is expressed in the
    /// old main-clock units, so it is dropped and the machine is asked to
    /// re-anchor its sub cycle target on the next slice.
    fn note_main_clock_change(&mut self) {
        self.sub_clock_credit = 0;
        self.clock_reanchor_pending = true;
    }

    /// Returns and clears the pending main-clock re-anchor request.
    pub(crate) fn take_clock_reanchor(&mut self) -> bool {
        core::mem::take(&mut self.clock_reanchor_pending)
    }

    /// Recomputes the effective sub clock, dividing it while the sub contends for
    /// VRAM without cycle steal, and re-anchors the accumulator on any change.
    fn update_sub_clock(&mut self) {
        let base = if self.clock_fast {
            self.model.sub_clock_hz()
        } else {
            self.model.sub_clock_slow_hz()
        };
        let effective = if !self.cycle_steal_enabled && self.vram_access_flag {
            (base / VRAM_CONTENTION_DIVISOR).max(1)
        } else {
            base
        };
        if effective != self.sub_clock_hz {
            self.sub_clock_hz = effective;
            self.sub_clock_credit = 0;
        }
    }

    /// The CLR clear-window auto-disarm delay in main-clock cycles.
    fn busy_clear_disarm_cycles(&self) -> u64 {
        let micros = if self.clock_fast {
            BUSY_CLEAR_DISARM_MICROS_FAST
        } else {
            BUSY_CLEAR_DISARM_MICROS_SLOW
        };
        self.micros_to_main_cycles(micros)
    }

    /// The delayed busy re-set delay in main-clock cycles.
    fn busy_delayed_set_cycles(&self) -> u64 {
        let micros = if self.clock_fast {
            BUSY_DELAYED_SET_MICROS_FAST
        } else {
            BUSY_DELAYED_SET_MICROS_SLOW
        };
        self.micros_to_main_cycles(micros)
    }

    /// Converts a microsecond delay into main-clock cycles, never below one cycle.
    fn micros_to_main_cycles(&self, micros: u64) -> u64 {
        (micros * u64::from(self.cpu_clock_hz()) / 1_000_000).max(1)
    }

    /// Arms the ALU busy-clear event `micros` microseconds out, replacing any
    /// previously pending clear.
    fn schedule_alu_busy_clear(&mut self, micros: u64) {
        let delay = self.micros_to_main_cycles(micros);
        self.scheduler.cancel(EventFm7::AluBusyClear);
        self.scheduler
            .schedule(EventFm7::AluBusyClear, self.current_cycle + delay);
    }

    /// Clears the ALU busy flag once a hardware line draw completes.
    fn clear_alu_busy(&mut self) {
        self.alu.clear_busy();
    }

    /// Schedules the next periodic sub display NMI event.
    fn schedule_sub_display_nmi(&mut self) {
        let period = self.micros_to_main_cycles(SUB_DISPLAY_NMI_PERIOD_MICROS);
        self.scheduler
            .schedule(EventFm7::SubDisplayNmi, self.current_cycle + period);
    }

    /// Schedules the next keyboard latch tick that drains the keycode FIFO.
    fn schedule_keyboard_latch(&mut self) {
        let period = self.micros_to_main_cycles(KEYBOARD_LATCH_PERIOD_MICROS);
        self.scheduler
            .schedule(EventFm7::KeyboardLatch, self.current_cycle + period);
    }

    /// Schedules the FM-77AV encoder ACK to be re-raised after its handshake
    /// delay, replacing any previously pending re-raise.
    fn schedule_encoder_ack(&mut self) {
        let delay = self.micros_to_main_cycles(ENCODER_ACK_DELAY_MICROS);
        self.scheduler
            .schedule(EventFm7::EncoderAck, self.current_cycle + delay);
    }

    /// Schedules the next FM-77AV RTC one-second tick.
    fn schedule_rtc_second(&mut self) {
        let period = self.micros_to_main_cycles(RTC_SECOND_PERIOD_MICROS);
        self.scheduler
            .schedule(EventFm7::RtcSecond, self.current_cycle + period);
    }

    /// Arms the FM-77AV auto-repeat timer `delay_ms` milliseconds out, replacing
    /// any previously pending repeat.
    fn schedule_keyboard_repeat(&mut self, delay_ms: u64) {
        let delay = self.micros_to_main_cycles(delay_ms * MICROS_PER_MILLI);
        self.scheduler
            .schedule(EventFm7::KeyboardRepeat, self.current_cycle + delay);
    }

    /// Reads the FM-77AV keyboard-encoder data register (`0xD431`), draining one
    /// response byte.
    pub(crate) fn encoder_read_data(&mut self) -> u8 {
        self.encoder.read_data()
    }

    /// Reads the FM-77AV keyboard-encoder status register (`0xD432`).
    pub(crate) fn encoder_read_status(&self) -> u8 {
        self.encoder.read_status()
    }

    /// Writes the FM-77AV keyboard-encoder data register (`0xD431`), feeding the
    /// byte to the encoder and scheduling its follow-up handshake and any RTC
    /// re-anchor the command requires.
    pub(crate) fn encoder_write_data(&mut self, value: u8) {
        let action = self.encoder.write_data(value);
        if action.schedule_ack {
            self.schedule_encoder_ack();
        }
        if action.reanchor_rtc {
            self.schedule_rtc_second();
        }
    }

    /// Sets the FM-77AV INSERT LED, driven by the sub `0xD40D` register.
    pub(crate) fn set_insert_led(&mut self, on: bool) {
        self.encoder.set_insert_led(on);
    }

    /// The FM-77AV keyboard LED status: bit 0 INSERT, bit 1 KANA, bit 2 CAPS.
    pub fn keyboard_led_status(&self) -> u8 {
        self.encoder.led_status()
    }

    /// The translated keycode table set the current encoder mode selects. The
    /// base FM-7 has no encoder and always uses the standard set.
    fn keycode_table_set(&self) -> KeycodeTableSet {
        if self.model.has_mmr() && self.encoder.scancode_mode() == ScancodeMode::Fm16Beta {
            KeycodeTableSet::Fm16Beta
        } else {
            KeycodeTableSet::Standard
        }
    }

    /// Generates one FM-77AV auto-repeat keystroke and re-arms the repeat timer.
    fn on_keyboard_repeat(&mut self) {
        if let Some(scancode) = self.encoder.repeat_scancode() {
            self.keyboard
                .enqueue_repeat(scancode, self.keycode_table_set());
            let interval = self.encoder.repeat_interval_ms();
            self.schedule_keyboard_repeat(interval);
        }
    }

    /// Starts or stops the FM-77AV encoder auto-repeat timer for a key event.
    fn update_auto_repeat(&mut self, scancode: u8, pressed: bool) {
        if pressed {
            if self.encoder.auto_repeat_active() && Fm7Keyboard::is_repeatable(scancode) {
                self.encoder.arm_repeat(scancode);
                let delay = self.encoder.repeat_delay_ms();
                self.schedule_keyboard_repeat(delay);
            }
        } else if self.encoder.cancel_repeat_if(scancode) {
            self.scheduler.cancel(EventFm7::KeyboardRepeat);
        }
    }

    /// Sets and immediately seeds the FM-77AV encoder RTC from the host time provider.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        let time = provider();
        self.encoder.seed_from_host(
            time.year,
            time.month,
            time.day,
            time.day_of_week,
            time.hour,
            time.minute,
            time.second,
        );
    }

    /// Schedules the next periodic timer IRQ event.
    fn schedule_timer(&mut self) {
        self.scheduler.schedule(
            EventFm7::TimerIrq,
            self.current_cycle + self.timer_irq_period_cycles(),
        );
    }

    /// Applies the averaged main-bus wait-state model for I/O-like accesses.
    ///
    /// In fast mode one wait cycle is inserted every second MMIO access. While
    /// the MMR or the relocatable window translates addresses the period
    /// alternates between three and two accesses, averaging one wait per 2.5
    /// accesses. Slow mode inserts no I/O waits.
    fn charge_main_access_wait(&mut self, address: u16) {
        if !self.clock_fast || !charges_io_wait(address) {
            return;
        }
        let period = if self.memory.mmr_translation_active() && self.io_wait_long_phase {
            IO_WAIT_PERIOD_MMR_LONG_ACCESSES
        } else {
            IO_WAIT_PERIOD_ACCESSES
        };
        self.io_wait_counter += 1;
        if self.io_wait_counter >= period {
            self.wait_cycles += 1;
            self.io_wait_counter = 0;
            self.io_wait_long_phase = !self.io_wait_long_phase;
        }
    }
}

/// Ephemeral `common::Bus` adapter for the main 6809.
pub struct MainBusView<'a, T: TraceSink = NoTrace> {
    /// Shared FM-7 bus.
    pub bus: &'a mut Fm7Bus<T>,
}

impl<T: TraceSink> common::Bus for MainBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, handled) = self.bus.read_byte(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Read,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address as u16;
        let handled = self.bus.write_byte(address, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, handled) = self.bus.read_byte(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Fetch,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
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
                    Some(u64::from(OPEN_BUS)),
                    false,
                ),
            );
        }
        0xFF
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
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
                    false,
                ),
            );
        }
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.bus.wait_cycles)
    }

    fn has_irq(&self) -> bool {
        self.bus.has_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let value = self.bus.acknowledge_irq();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::FM7_MAIN_IRQ,
                    0,
                    TraceInterruptAction::Acknowledge,
                    None,
                ),
            );
        }
        value
    }

    fn has_nmi(&self) -> bool {
        false
    }

    fn acknowledge_nmi(&mut self) {}

    fn acknowledge_firq(&mut self) {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::FM7_MAIN_FIRQ,
                    0,
                    TraceInterruptAction::Acknowledge,
                    None,
                ),
            );
        }
    }

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

/// Ephemeral `common::Bus` adapter for the display sub 6809.
///
/// The sub CPU runs in its own clock domain (`sub_cycle`) so it never perturbs the
/// scheduler timebase driven by the main CPU.
pub struct SubBusView<'a, T: TraceSink = NoTrace> {
    /// Shared FM-7 bus.
    pub bus: &'a mut Fm7Bus<T>,
}

impl<T: TraceSink> common::Bus for SubBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, handled) = self.bus.read_sub_space_byte(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Read,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address as u16;
        let handled = self.bus.write_sub_space_byte(address, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, handled) = self.bus.read_sub_space_byte(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_MEMORY,
                    TraceAccessKind::Fetch,
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_IO,
                    TraceAccessKind::Read,
                    u64::from(port),
                    TraceAccessWidth::Byte,
                    Some(u64::from(OPEN_BUS)),
                    false,
                ),
            );
        }
        OPEN_BUS
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::SUB_IO,
                    TraceAccessKind::Write,
                    u64::from(port),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    false,
                ),
            );
        }
    }

    fn has_irq(&self) -> bool {
        self.bus.sub_has_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let value = self.bus.acknowledge_sub_irq();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::FM7_SUB_IRQ,
                    0,
                    TraceInterruptAction::Acknowledge,
                    None,
                ),
            );
        }
        value
    }

    fn has_nmi(&self) -> bool {
        self.bus.sub_has_nmi()
    }

    fn acknowledge_nmi(&mut self) {
        self.bus.acknowledge_sub_nmi();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::FM7_SUB_NMI,
                    common::TraceInterruptKind::NonMaskable,
                    None,
                    TraceInterruptAction::Acknowledge,
                    None,
                ),
            );
        }
    }

    fn acknowledge_firq(&mut self) {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::FM7_SUB_FIRQ,
                    0,
                    TraceInterruptAction::Acknowledge,
                    None,
                ),
            );
        }
    }

    #[allow(clippy::misnamed_getters)]
    fn current_cycle(&self) -> u64 {
        self.bus.sub_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.bus.sub_cycle = cycle;
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.bus.tracer.yield_requested()
    }
}

/// Whether an access receives the averaged I/O wait-state charge.
fn charges_io_wait(address: u16) -> bool {
    matches!(address, MAIN_IO_START..=MAIN_IO_END | VECTOR_WAIT_START..=VECTOR_WAIT_END)
}

#[cfg(test)]
mod tests {
    use common::{Bus, TraceAccess};

    use super::*;

    #[derive(Default)]
    struct AccessTrace {
        accesses: Vec<TraceAccess>,
    }

    impl TraceSink for AccessTrace {
        fn trace(&mut self, _context: TraceContext, event: TraceEvent<'_>) {
            if let TraceEvent::Access(access) = event {
                self.accesses.push(access);
            }
        }
    }

    /// Confirms unused main and sub CPU ports retain their open-bus values.
    #[test]
    fn unused_cpu_ports_trace_open_bus_values() {
        let mut bus = Fm7Bus::new_with_trace_sink(
            Fm7Model::Fm7,
            BootMode::Basic,
            48_000,
            AccessTrace::default(),
        );

        let main_value = Bus::io_read_byte(&mut MainBusView { bus: &mut bus }, 0x1234);
        let sub_value = Bus::io_read_byte(&mut SubBusView { bus: &mut bus }, 0x5678);

        assert_eq!(main_value, OPEN_BUS);
        assert_eq!(sub_value, OPEN_BUS);
        assert_eq!(bus.tracer().accesses.len(), 2);
        assert_eq!(bus.tracer().accesses[0].space, TraceAddressSpace::MAIN_IO);
        assert_eq!(bus.tracer().accesses[1].space, TraceAddressSpace::SUB_IO);
        assert!(bus.tracer().accesses.iter().all(|access| {
            access.kind == TraceAccessKind::Read
                && access.width == TraceAccessWidth::Byte
                && access.value == Some(u64::from(OPEN_BUS))
                && !access.handled
        }));
    }
}
