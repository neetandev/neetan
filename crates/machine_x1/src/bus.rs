//! Sharp X1 system bus.
//!
//! A single-Z80 machine: the bus owns the memory map, the video/CRTC, the Z80
//! CTC, the main PPI, the AY-3-8910 PSG, the MB8877 FDC, the cassette deck, the
//! HLE sub-CPU, the event scheduler and a monotonic `current_cycle` in
//! main-clock units.

mod dma;
mod fdc;
mod io_read;
mod io_wait;
mod io_write;
mod ppi_link;

use common::{
    JoystickState, MonitorTiming, NoTrace, SharedHostDateTimeSource, TraceAccessKind,
    TraceAccessWidth, TraceAddressSpace, TraceContext, TraceEvent, TraceInterruptAction,
    TraceInterruptKind, TracePresentation, TraceSink, trace_id,
};
use device::{
    cassette::{CassetteDeck, CassetteError, load_cassette},
    hd6845_crtc::Hd6845,
    mouse_x1::MouseX1,
    opn_fm::FmTimerAction,
    psg::Ay38910,
    soundboard_opm::SoundBoardOpm,
    subcontroller_x1::{CassetteAction, SubHle},
    video_x1::{
        MODE1_ANK16, MODE1_CG_STRIDE_400, MODE1_CHAR_CLOCK_15, MODE1_KANJI_UNDERLINE, X1Video,
    },
    wd17xx_fdc::{WD17XX_PLATFORM_X1, Wd17xxFdc},
    z80_ctc::Z80Ctc,
    z80_dma::Z80Dma,
    z80_sio::Z80Sio,
};
use ppi_link::PpiLink;
use software_renderer::x1::{RenderInputsX1, X1DebugLayer, X1Renderer, X1RendererModel};

use crate::{
    config::{ClockConfig, X1KeyboardMode, X1Model},
    interrupt::{InterruptController, IrqSource},
    memory::X1Memory,
    scheduler::{EventX1, X1Scheduler},
};

/// Base X1 display resolution.
const DISPLAY_WIDTH: u32 = 640;
const DISPLAY_HEIGHT: u32 = 200;

/// Vertical refresh rate in Hz.
const REFRESH_HZ: u64 = 60;

/// AY-3-8910 PSG input clock (MAIN_CLOCK / 8 = 16 MHz / 8).
const PSG_CLOCK_HZ: u32 = 2_000_000;

/// Base-X1 horizontal period in CPU cycles (15 kHz class); used to derive the
/// beam position for PCG addressing and the V-DISP / V-SYNC flags.
const HORIZONTAL_PERIOD_CYCLES: u64 = 250;
/// X1 turbo hi-res horizontal period in CPU cycles (24 kHz class).
const HORIZONTAL_PERIOD_CYCLES_HIRES: u64 = 161;

/// Sub-CPU mailbox poll period in microseconds.
const SUB_POLL_MICROS: u64 = 400;

/// PPI port B bit assignments.
const PORT_B_VDISP: u8 = 0x80;
const PORT_B_RAM_BANK: u8 = 0x10;
const PORT_B_VSYNC: u8 = 0x04;
const PORT_B_CASSETTE: u8 = 0x02;
const PORT_B_BREAK: u8 = 0x01;

/// Value returned for reads of unmapped I/O ports.
const OPEN_BUS: u8 = 0xFF;

/// Wait cycles for accesses to the sub-CPU mailbox (`0x1900`) and PSG
/// (`0x1B00`/`0x1C00`) ports.
const SUB_CPU_PSG_WAIT_CYCLES: i64 = 1;

/// Offset of the 8x16 glyphs inside the ANK font ROM.
const ANK16_ROM_OFFSET: usize = 0x1000;

/// Wait cycles per M1 opcode fetch on the base X1 while the IPL ROM is mapped.
const M1_ROM_FETCH_WAIT_CYCLES: i64 = 1;

/// CRTC vertical totals above this many raster lines select the 24 kHz hi-res
/// scan and its VRAM wait tables.
const HIRES_SCAN_LINE_THRESHOLD: u32 = 400;

/// Translates a host key sub-id (0x00-0x7F) to the sub-CPU virtual-key code.
/// Most ids map to themselves; the punctuation and symbol keys whose native
/// virtual-key codes exceed 0x7F are assigned spare low ids so the host release
/// flag (bit 7) never collides with a key code.
fn subid_to_virtual_key(subid: u8) -> u8 {
    match subid {
        0x01 => 0xBA,
        0x02 => 0xBB,
        0x04 => 0xBC,
        0x05 => 0xBD,
        0x06 => 0xBE,
        0x07 => 0xBF,
        0x0A => 0xC0,
        0x0B => 0xDB,
        0x0C => 0xDC,
        0x0E => 0xDD,
        0x0F => 0xDE,
        0x16 => 0xE8,
        other => other,
    }
}

impl X1Bus<NoTrace> {
    /// Creates an untraced bus for `model` at the given audio sample rate.
    pub fn new(model: X1Model, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(model, sample_rate, NoTrace)
    }
}

save_state::runtime_state! {
/// CRTC display parameters latched once per frame (and re-latched when the
/// timing registers are reprogrammed), so the whole frame renders with one
/// consistent geometry.
#[derive(Clone)]
struct FrameCrtcParams {
    /// Scanlines per character row (R9 + 1).
    ch_height: u16,
    /// Total character columns per scanline (R0 + 1).
    hz_total: u16,
    /// Displayed character columns (R1).
    hz_disp: u16,
    /// Displayed character rows (R6).
    vt_disp: u16,
    /// Display-memory start address (R12/R13).
    st_addr: u16,
    /// Blanked lines above the display (R5 with model adjustments).
    vt_ofs: u16,
    /// Whether the frame runs the 24 kHz hi-res scan (vertical total > 400).
    hires: bool,
    /// Total scanlines per frame, when the CRTC is programmed sensibly.
    total_lines: Option<u64>,
}}

save_state::runtime_state! {
/// Complete authoritative Sharp X1 family bus state.
#[derive(Clone)]
pub(crate) struct X1BusState {
    memory: crate::memory::X1MemoryState,
    scheduler: common::SchedulerState,
    interrupt: crate::interrupt::InterruptController,
    crtc: device::hd6845_crtc::Hd6845State,
    ctc: device::z80_ctc::Z80Ctc,
    ppi: crate::bus::ppi_link::PpiLinkState,
    psg: device::psg::PsgState,
    fm: Option<device::opn_fm::OpnFmState<ymfm_oxide::Ym2151, ymfm_oxide::YmfmOutput2>>,
    sound_ctc: device::z80_ctc::Z80Ctc,
    fdc: device::wd17xx_fdc::Wd17xxFdcState,
    dma: device::z80_dma::Z80DmaState,
    sio: device::z80_sio::Z80Sio,
    mouse: device::mouse_x1::MouseX1,
    cassette: device::cassette::CassetteDeckState,
    sub: device::subcontroller_x1::SubHle,
    video: device::video_x1::X1Video,
    renderer: software_renderer::x1::X1RendererState,
    kanji_address_latch: u16,
    kanji_glyph_base: usize,
    kanji_read_flags: u8,
    kanji_read_row: u8,
    joystick_player_one: u8,
    joystick_player_two: u8,
    wait_cycles: i64,
    vram_wait_remainder: i64,
    dma_stall_deadline: u64,
    current_cycle: u64,
    presented_frames: u64,
    frame_start_cycle: u64,
    frame_number: u32,
    frame_params: FrameCrtcParams,
    vblank_anchor_cycle: u64,
    port_b_vdisp_seen: bool,
    character_blink: u8,
    rtc_accumulator: u64,
    column_40: bool,
    display_width: u32,
    display_height: u32,
}}

/// The Sharp X1 system bus.
pub struct X1Bus<T: TraceSink = NoTrace> {
    model: X1Model,
    clocks: ClockConfig,
    /// Selected monitor, reported through the turbo DIP switch so software knows
    /// whether a 24 kHz (400-line) display is attached.
    monitor_timing: MonitorTiming,
    memory: X1Memory,
    /// Event scheduler.
    pub(crate) scheduler: X1Scheduler,
    interrupt: InterruptController,
    crtc: Hd6845,
    ctc: Z80Ctc,
    ppi: PpiLink,
    psg: Ay38910,
    /// CZ-8BS1 FM sound board (YM2151); present only on the turbo.
    fm: Option<SoundBoardOpm>,
    /// Sound-board Z80 CTC (`ctc_ym`); used only when the FM board is present.
    sound_ctc: Z80Ctc,
    fdc: Wd17xxFdc<WD17XX_PLATFORM_X1>,
    dma: Z80Dma,
    sio: Z80Sio,
    mouse: MouseX1,
    cassette: CassetteDeck,
    sub: SubHle,
    video: X1Video,
    renderer: X1Renderer,
    cg_rom: Vec<u8>,
    ank_rom: Vec<u8>,
    kanji_rom: Vec<u8>,
    rom_bindings: Vec<save_state::ResourceBinding>,
    /// Kanji data-port address latch (`0x0E80` low byte, `0x0E81` high byte).
    kanji_address_latch: u16,
    /// Kanji ROM offset of the glyph latched by an `0x0E82` write.
    kanji_glyph_base: usize,
    /// Which glyph halves of the current row were read: bit 0 left, bit 1 right.
    kanji_read_flags: u8,
    /// Glyph row (0-15) returned by the kanji data ports; advances once both
    /// halves of the row were read.
    kanji_read_row: u8,
    joystick_p1: u8,
    joystick_p2: u8,
    /// Wait-state cycles accumulated by VRAM, mailbox/PSG and M1 accesses,
    /// drained by the CPU after each instruction.
    wait_cycles: i64,
    /// Sub-cycle fractional carry for the bitmap VRAM wait average, in units of
    /// `1 / io_wait::VRAM_WAIT_PERIOD` of a cycle. Kept in `[0, VRAM_WAIT_PERIOD)`
    /// and persisted across CPU steps so the long-run average stall is exact.
    vram_wait_remainder: i64,
    /// Cycle past which a continuous-mode DMA transfer must not keep stalling
    /// the CPU within the current [`crate::X1Machine::run_for`] call. Set to the
    /// run budget's target so a long transfer is sliced across audio steps
    /// instead of overrunning one; `u64::MAX` (the default) leaves the transfer
    /// unbounded for event-driven callers such as tests.
    dma_stall_deadline: u64,
    current_cycle: u64,
    frame_start_cycle: u64,
    frame_number: u32,
    /// Number of frames presented within the current automation epoch.
    presented_frames: u64,
    /// Fractional audio-sample carry for deterministic automation audio draining.
    automation_audio_remainder: u64,
    /// CRTC geometry latched for the current frame.
    frame_params: FrameCrtcParams,
    /// Cycle at which the current vertical blanking period began; the beam
    /// position for PCG addressing counts from here.
    vblank_anchor_cycle: u64,
    /// The V-DISP bit as last observed by a CPU read of the PPI port B. A read
    /// that sees the bit drop re-anchors the vertical blanking clock, keeping
    /// beam-timed PCG accesses aligned with software that polls for blanking.
    port_b_vdisp_seen: bool,
    /// Frame counter driving the text blink attribute (phase bit 5).
    cblink: u8,
    rtc_accumulator: u64,
    /// 320-pixel / 40-column hi-speed pixel-clock mode (PPI port C bit 6). When
    /// set the pixel clock is halved, so the display is stretched to twice its
    /// horizontal size to fill the screen.
    column40: bool,
    display_width: u32,
    display_height: u32,
    /// Bus-activity tracer (a no-op by default).
    tracer: T,
}

impl<T: TraceSink> X1Bus<T> {
    /// Creates a traced bus for `model` at the given audio sample rate.
    pub fn new_with_trace_sink(model: X1Model, sample_rate: u32, tracer: T) -> Self {
        let clocks = ClockConfig {
            main_clock_hz: model.main_clock_hz(),
            sample_rate,
        };
        let mut scheduler = X1Scheduler::new();
        scheduler.schedule(
            EventX1::VBlank,
            u64::from(clocks.main_clock_hz) / REFRESH_HZ,
        );
        scheduler.schedule(EventX1::Scanline, HORIZONTAL_PERIOD_CYCLES);
        let sub_poll = sub_poll_period(clocks.main_clock_hz);
        scheduler.schedule(EventX1::SubPoll, sub_poll);
        let crtc = Hd6845::new();
        let frame_params = latch_frame_params(&crtc, model, 0);
        let mut psg = Ay38910::new();
        psg.configure_audio(PSG_CLOCK_HZ, clocks.main_clock_hz, sample_rate);
        Self {
            model,
            clocks,
            monitor_timing: MonitorTiming::default(),
            memory: X1Memory::new(model),
            scheduler,
            interrupt: InterruptController::new(),
            crtc,
            ctc: Z80Ctc::new(),
            ppi: PpiLink::new(),
            psg,
            fm: if model.has_fm() {
                Some(SoundBoardOpm::new(clocks.main_clock_hz, sample_rate))
            } else {
                None
            },
            sound_ctc: Z80Ctc::new(),
            fdc: Wd17xxFdc::new(clocks.main_clock_hz),
            dma: Z80Dma::new(),
            sio: Z80Sio::new(),
            mouse: MouseX1::new(),
            cassette: CassetteDeck::new(),
            sub: SubHle::new(model.is_turbo(), clocks.main_clock_hz),
            video: X1Video::new(),
            renderer: X1Renderer::new(&[]),
            cg_rom: Vec::new(),
            ank_rom: Vec::new(),
            kanji_rom: Vec::new(),
            rom_bindings: Vec::new(),
            kanji_address_latch: 0,
            kanji_glyph_base: 0,
            kanji_read_flags: 0,
            kanji_read_row: 0,
            joystick_p1: 0xFF,
            joystick_p2: 0xFF,
            wait_cycles: 0,
            vram_wait_remainder: 0,
            dma_stall_deadline: u64::MAX,
            current_cycle: 0,
            frame_start_cycle: 0,
            frame_number: 0,
            presented_frames: 0,
            automation_audio_remainder: 0,
            frame_params,
            vblank_anchor_cycle: 0,
            port_b_vdisp_seen: false,
            cblink: 0,
            rtc_accumulator: 0,
            column40: false,
            display_width: DISPLAY_WIDTH,
            display_height: DISPLAY_HEIGHT,
            tracer,
        }
    }

    /// Loads the ROM set into memory and the renderer font.
    pub fn load_roms(&mut self, roms: &crate::rom::LoadedRoms) {
        self.memory.load_ipl(&roms.ipl);
        self.cg_rom = roms.cgrom_8x8.clone();
        self.ank_rom = roms.ank.clone();
        self.kanji_rom = roms.kanji.clone().unwrap_or_default();
        self.renderer.update_font(&roms.cgrom_8x8);
        self.rom_bindings.clear();
        for (identifier, bytes) in [
            ("ipl", Some(roms.ipl.as_slice())),
            ("cg", Some(roms.cgrom_8x8.as_slice())),
            ("ank", Some(roms.ank.as_slice())),
            ("kanji", roms.kanji.as_deref()),
        ] {
            if let Some(bytes) = bytes {
                self.rom_bindings.push(save_state::ResourceBinding {
                    identifier: save_state::ResourceBindingId::new(format!("rom:{identifier}"))
                        .expect("static resource identifier"),
                    identity: save_state::ResourceIdentity::from_bytes(bytes),
                });
            }
        }
    }

    /// Selects the attached monitor, reported through the turbo DIP switch.
    pub fn set_monitor_timing(&mut self, timing: MonitorTiming) {
        self.monitor_timing = timing;
    }

    /// Selects the turbo keyboard's mode switch position (A or B).
    pub fn set_keyboard_mode(&mut self, mode: X1KeyboardMode) {
        self.sub.set_keyboard_mode(mode);
    }

    /// Value returned by the turbo DIP-switch port (`0x1FF0`).
    ///
    /// Bit 0 is the monitor type (0 = high-resolution 24 kHz, 1 = standard 15 kHz),
    /// bits 3:1 the default auto-boot device (0 = 2D, 2 = 2DD, 4 = 2HD), and bits
    /// 7:4 read back clear. Software reads this to decide between 200- and 400-line
    /// modes. Only read on turbo models; the base X1 has no DIP port.
    pub(crate) fn dip_switch(&self) -> u8 {
        const BOOT_DEVICE_2D: u8 = 0x00;
        let monitor_bit = match self.monitor_timing {
            MonitorTiming::Auto | MonitorTiming::Fixed24kHz => 0x00,
            MonitorTiming::Fixed15kHz => 0x01,
        };
        monitor_bit | BOOT_DEVICE_2D
    }

    /// The main CPU clock in Hz.
    pub fn cpu_clock_hz(&self) -> u32 {
        self.clocks.main_clock_hz
    }

    /// Returns the configured X1 model.
    pub fn model(&self) -> X1Model {
        self.model
    }

    pub(crate) fn save_state_resources(
        &self,
    ) -> Result<save_state::ResourceManifest, save_state::StateValidationError> {
        save_state::ResourceManifest::new(self.rom_bindings.clone())
    }

    pub(crate) fn save_state_media(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = self.fdc.media_manifest()?.bindings().to_vec();
        if let Some(identity) = self.cassette.media_identity() {
            bindings.push(save_state::MediaBinding {
                identifier: save_state::MediaBindingId::new("cassette-0")?,
                slot: save_state::MediaSlot::new(save_state::MediaKind::Cassette, 0),
                source_path: self.cassette.media_source_path().cloned(),
                media_type: "cassette".to_owned(),
                identity,
                geometry: None,
                write_protected: true,
                backend_generation: None,
            });
        }
        save_state::MediaManifest::new(bindings)
    }

    pub(crate) fn capture_runtime_state(&self) -> Result<X1BusState, save_state::SaveStateError> {
        Ok(X1BusState {
            memory: self.memory.capture_state(),
            scheduler: self.scheduler.capture_state(),
            interrupt: self.interrupt.clone(),
            crtc: self.crtc.state.clone(),
            ctc: self.ctc.capture_state(),
            ppi: self.ppi.capture_state(),
            psg: self.psg.capture_state(),
            fm: self.fm.as_ref().map(SoundBoardOpm::capture_state),
            sound_ctc: self.sound_ctc.capture_state(),
            fdc: self.fdc.capture_state()?,
            dma: self.dma.capture_state(),
            sio: self.sio.capture_state(),
            mouse: self.mouse.clone(),
            cassette: self.cassette.capture_state(),
            sub: self.sub.capture_state(),
            video: self.video.capture_state(),
            renderer: self.renderer.capture_state(),
            kanji_address_latch: self.kanji_address_latch,
            kanji_glyph_base: self.kanji_glyph_base,
            kanji_read_flags: self.kanji_read_flags,
            kanji_read_row: self.kanji_read_row,
            joystick_player_one: self.joystick_p1,
            joystick_player_two: self.joystick_p2,
            wait_cycles: self.wait_cycles,
            vram_wait_remainder: self.vram_wait_remainder,
            dma_stall_deadline: self.dma_stall_deadline,
            current_cycle: self.current_cycle,
            presented_frames: self.presented_frames,
            frame_start_cycle: self.frame_start_cycle,
            frame_number: self.frame_number,
            frame_params: self.frame_params.clone(),
            vblank_anchor_cycle: self.vblank_anchor_cycle,
            port_b_vdisp_seen: self.port_b_vdisp_seen,
            character_blink: self.cblink,
            rtc_accumulator: self.rtc_accumulator,
            column_40: self.column40,
            display_width: self.display_width,
            display_height: self.display_height,
        })
    }

    pub(crate) fn restore_runtime_state(
        &mut self,
        state: X1BusState,
    ) -> Result<(), save_state::SaveStateError> {
        if state.kanji_read_flags > 3
            || state.kanji_read_row >= 16
            || state.vram_wait_remainder < 0
            || state.vram_wait_remainder >= io_wait::VRAM_WAIT_PERIOD
            || !(1..=640).contains(&state.display_width)
            || !(1..=400).contains(&state.display_height)
            || state.frame_params.ch_height == 0
            || state.frame_params.ch_height > 32
            || state.frame_params.hz_total == 0
            || state.frame_params.hz_total > 256
            || state.frame_params.st_addr > 0x3FFF
            || state.frame_params.vt_ofs > 31
            || state
                .frame_params
                .total_lines
                .is_some_and(|lines| lines == 0 || lines > 1024)
        {
            return Err(
                save_state::StateValidationError::new("X1 state invariant is invalid").into(),
            );
        }
        state.interrupt.validate_runtime_state()?;
        match (&mut self.fm, state.fm) {
            (Some(sound), Some(state)) => sound.restore_state(state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "X1 FM board configuration differs",
                )
                .into());
            }
        }
        self.memory.restore_state(state.memory)?;
        self.ctc.restore_state(state.ctc)?;
        self.sound_ctc.restore_state(state.sound_ctc)?;
        self.fdc.restore_state(state.fdc)?;
        self.dma.restore_state(state.dma)?;
        self.sio.restore_state(state.sio)?;
        self.cassette.restore_state(state.cassette)?;
        self.sub.restore_state(state.sub)?;
        self.renderer.restore_state(state.renderer)?;
        self.psg.restore_state(state.psg)?;
        self.scheduler.restore_state(state.scheduler)?;
        self.interrupt = state.interrupt;
        self.crtc.state = state.crtc;
        self.ppi.restore_state(state.ppi);
        self.mouse = state.mouse;
        self.video.restore_state(state.video);
        self.kanji_address_latch = state.kanji_address_latch;
        self.kanji_glyph_base = state.kanji_glyph_base;
        self.kanji_read_flags = state.kanji_read_flags;
        self.kanji_read_row = state.kanji_read_row;
        self.joystick_p1 = state.joystick_player_one;
        self.joystick_p2 = state.joystick_player_two;
        self.wait_cycles = state.wait_cycles;
        self.vram_wait_remainder = state.vram_wait_remainder;
        self.dma_stall_deadline = state.dma_stall_deadline;
        self.current_cycle = state.current_cycle;
        self.presented_frames = state.presented_frames;
        self.frame_start_cycle = state.frame_start_cycle;
        self.frame_number = state.frame_number;
        self.frame_params = state.frame_params;
        self.vblank_anchor_cycle = state.vblank_anchor_cycle;
        self.port_b_vdisp_seen = state.port_b_vdisp_seen;
        self.cblink = state.character_blink;
        self.rtc_accumulator = state.rtc_accumulator;
        self.column40 = state.column_40;
        self.display_width = state.display_width;
        self.display_height = state.display_height;
        self.sync_interrupts();
        Ok(())
    }

    /// The current monotonic cycle count (main-clock units).
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Advances the monotonic cycle counter.
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    /// Sets the deadline past which a continuous-mode DMA transfer stops
    /// stalling the CPU for the current run, deferring the remainder. Pass
    /// `u64::MAX` to leave transfers unbounded.
    pub fn set_dma_stall_deadline(&mut self, deadline: u64) {
        self.dma_stall_deadline = deadline;
    }

    /// The cycle of the next scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// A shared reference to the bus-activity tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// A mutable reference to the bus-activity tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Reads a byte through the active memory map (for tests and tooling).
    pub fn peek_byte(&self, address: u16) -> u8 {
        self.memory.read(address)
    }

    /// Writes a byte through the active memory map (for tests and tooling).
    pub fn poke_byte(&mut self, address: u16, value: u8) {
        self.memory.write(address, value);
    }

    /// Whether the IPL ROM is currently mapped in the bottom 32 KiB.
    pub fn rom_selected(&self) -> bool {
        self.memory.rom_selected()
    }

    /// The last rendered framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// The framebuffer dimensions.
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// Renders a diagnostic layer into the framebuffer and returns its
    /// dimensions.
    pub fn render_debug_layer(&mut self, layer: X1DebugLayer) -> (u32, u32) {
        let inputs = RenderInputsX1 {
            text_vram: self.video.text_vram(),
            attr_vram: self.video.attr_vram(),
            kvram: self.video.kvram(),
            pcg: self.video.pcg(),
            gaiji: self.video.pcg_gaiji(),
            ank_rom: &self.ank_rom,
            kanji_rom: &self.kanji_rom,
            bitmap: self.video.bitmap(),
            palette: self.video.palette_guns(),
            priority: self.video.priority(),
            mode1: self.video.mode1(),
            mode2: self.video.mode2(),
            cblink: self.cblink,
            display_off: (self.crtc.state.regs[8] & 0x30) == 0x30,
            hz_disp: self.frame_params.hz_disp,
            vt_disp: self.frame_params.vt_disp,
            ch_height: self.frame_params.ch_height,
            st_addr: self.frame_params.st_addr,
            vt_ofs: self.frame_params.vt_ofs,
            hires: self.frame_params.hires,
            column40: self.column40,
            model: renderer_model(self.model),
        };
        self.renderer.render_debug_layer(&inputs, layer)
    }

    /// The character-generator ROM data (for host-side font tooling).
    pub fn font_rom_data(&self) -> &[u8] {
        &self.cg_rom
    }

    /// Injects a host keyboard event. The byte is a host key sub-id with bit 7
    /// set on release.
    pub fn push_keyboard_scancode(&mut self, code: u8) {
        let pressed = (code & 0x80) == 0;
        let virtual_key = subid_to_virtual_key(code & 0x7F);
        if pressed {
            self.sub.key_down(virtual_key, false, self.current_cycle);
        } else {
            self.sub.key_up(virtual_key);
        }
        self.sync_interrupts();
    }

    /// Updates the primary joystick state (AY port A / P1).
    pub fn set_joystick(&mut self, state: JoystickState) {
        self.joystick_p1 = joystick_to_port(state);
    }

    /// Feeds host mouse movement and button state to the turbo mouse. Movement is
    /// accumulated until the next report; buttons are a live bitmask (bit 0 =
    /// left, bit 1 = right).
    pub fn set_mouse_input(&mut self, delta_x: i32, delta_y: i32, buttons: u8) {
        self.mouse.add_movement(delta_x, delta_y);
        self.mouse.set_buttons(buttons);
    }

    /// Injects a byte into the RS-232C receiver (SIO channel 0) as if it arrived
    /// on the serial line, refreshing the interrupt. Exercises the receive path in
    /// tests and gives a future host serial backend its entry point.
    pub fn push_rs232c_received_byte(&mut self, byte: u8) {
        self.sio.receive(0, byte);
        self.sync_interrupts();
    }

    /// Font read through the PCG font port (`0x1400`) in the direct-access
    /// mode: the character comes from the staged cell (the first magic cell
    /// whose attribute clears the PCG-select bit), and the glyph row from the
    /// port's low bits. Kanji cells read the kanji ROM, otherwise the 8x16 ANK
    /// font when mode register 1 selects it, otherwise the 8x8 CG-ROM.
    pub(crate) fn pcg_direct_font_read(&self, port: u16) -> u8 {
        let cell = self.video.check_char_address();
        let ank = self.video.read_text(cell);
        let kanji_attr = self.video.read_kvram(cell);
        if kanji_attr & 0x80 != 0 {
            let code = (u16::from(kanji_attr) << 8) | u16::from(ank);
            let side = usize::from((kanji_attr & 0x40) >> 2);
            let offset = usize::from(code & 0x0FFF) * 0x20 + usize::from(port & 0x0F) + side;
            self.kanji_rom.get(offset).copied().unwrap_or(0xFF)
        } else if self.video.mode1() & MODE1_ANK16 != 0 {
            let offset = ANK16_ROM_OFFSET + usize::from(ank) * 16 + usize::from(port & 0x0F);
            self.ank_rom.get(offset).copied().unwrap_or(0xFF)
        } else {
            let index = (usize::from(ank) * 8 + usize::from((port >> 1) & 7)) & 0x7FF;
            self.cg_rom.get(index).copied().unwrap_or(0)
        }
    }

    /// Reads a kanji data port (`0x0E80` left half, `0x0E81` right half). With a
    /// glyph latched (address high byte nonzero) the ports return the current
    /// glyph row of their half; the row advances once both halves were read.
    /// With the high byte zero, `0x0E80` returns the high byte of the kanji-port
    /// address for the JIS row in the latch low byte, and `0x0E81` returns zero.
    pub(crate) fn kanji_data_read(&mut self, offset: u16) -> u8 {
        let glyph_latched = self.kanji_address_latch & 0xFF00 != 0;
        if offset == 0 {
            if glyph_latched {
                let index = self.kanji_glyph_base + usize::from(self.kanji_read_row);
                let value = self.kanji_rom.get(index).copied().unwrap_or(0xFF);
                self.kanji_read_flags |= 1;
                self.advance_kanji_row_if_complete();
                value
            } else {
                (jis_row_address(self.kanji_address_latch as u8) >> 8) as u8
            }
        } else if glyph_latched {
            let index = self.kanji_glyph_base + usize::from(self.kanji_read_row) + 16;
            let value = self.kanji_rom.get(index).copied().unwrap_or(0xFF);
            self.kanji_read_flags |= 2;
            self.advance_kanji_row_if_complete();
            value
        } else {
            0
        }
    }

    fn advance_kanji_row_if_complete(&mut self) {
        if self.kanji_read_flags == 3 {
            self.kanji_read_flags = 0;
            self.kanji_read_row = (self.kanji_read_row + 1) & 15;
        }
    }

    /// Writes a kanji address-latch / select port (`0x0E80`-`0x0E82`). Writing
    /// the select port latches the glyph for the data-port reads.
    pub(crate) fn kanji_write(&mut self, offset: u16, value: u8) {
        match offset {
            0 => {
                self.kanji_address_latch = (self.kanji_address_latch & 0xFF00) | u16::from(value);
            }
            1 => {
                self.kanji_address_latch =
                    (u16::from(value) << 8) | (self.kanji_address_latch & 0x00FF);
            }
            2 => {
                self.kanji_glyph_base = jis_convert(self.kanji_address_latch & 0xFFF0);
            }
            _ => {}
        }
    }

    /// Delivers a mouse report into SIO channel 1 when channel 1's RTS output
    /// makes a high-to-low transition (the mouse read handshake).
    fn poll_mouse_rts(&mut self) {
        if self.sio.take_rts_falling_edge(1) {
            let report = self.mouse.report();
            self.sio.clear_receive(1);
            for byte in report {
                self.sio.receive(1, byte);
            }
        }
    }

    /// Generates paced audio for the elapsed cycles, returning the number of
    /// `f32` values written. Output is stereo interleaved.
    pub(crate) fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        // The PSG is the base writer (it overwrites the buffer).
        let count = self.psg.generate_samples(
            self.current_cycle,
            PSG_CLOCK_HZ,
            self.clocks.main_clock_hz,
            self.clocks.sample_rate,
            volume,
            output,
        );
        // The FM board (when present) mixes additively on top.
        if let Some(fm) = &mut self.fm {
            fm.generate_samples(
                self.current_cycle,
                self.clocks.main_clock_hz,
                volume,
                output,
            );
        }
        self.apply_fm_timers();
        count
    }

    /// Processes all events due at the current cycle.
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
                EventX1::VBlank => {
                    self.present_latched_frame();
                    self.trace_presentation();
                    self.frame_number = self.frame_number.wrapping_add(1);
                    self.cblink = (self.cblink + 1) & 0x3F;
                    self.frame_start_cycle = event.fire_cycle;
                    self.latch_frame_params();
                    let period = self.frame_period();
                    self.renderer.clear_latched_frame();
                    self.scheduler
                        .schedule(EventX1::VBlank, event.fire_cycle + period);
                    self.schedule_next_scanline_after(event.fire_cycle);
                }
                EventX1::VSync => {}
                EventX1::Scanline => {
                    self.anchor_vblank_if_crossed(event.fire_cycle);
                    self.latch_scanline(event.fire_cycle);
                    self.schedule_next_scanline_after(event.fire_cycle);
                }
                EventX1::CtcChannel0 => self.on_ctc_zero(0, event.fire_cycle),
                EventX1::CtcChannel1 => self.on_ctc_zero(1, event.fire_cycle),
                EventX1::CtcChannel2 => self.on_ctc_zero(2, event.fire_cycle),
                EventX1::CtcChannel3 => self.on_ctc_zero(3, event.fire_cycle),
                EventX1::DmaTick => self.do_dma(),
                EventX1::FdcSeekComplete => self.on_fdc_seek_complete(event.fire_cycle),
                EventX1::KeyScan => {}
                EventX1::CassetteByte => {}
                EventX1::SubPoll => {
                    let period = sub_poll_period(self.clocks.main_clock_hz);
                    self.sub.set_interrupt_enabled_line(
                        !self.interrupt.higher_in_service(IrqSource::Keyboard),
                    );
                    self.sub.poll(event.fire_cycle);
                    self.accumulate_rtc(period);
                    if self.sub.take_rtc_phase_reset() {
                        self.rtc_accumulator = 0;
                    }
                    self.advance_cassette();
                    self.auto_stop_cassette();
                    if let Some(action) = self.sub.take_cassette_action() {
                        self.apply_cassette_action(action);
                    }
                    self.sync_interrupts();
                    self.scheduler
                        .schedule(EventX1::SubPoll, event.fire_cycle + period);
                }
                EventX1::SioTxCh0 | EventX1::SioRxCh0 | EventX1::SioTxCh1 | EventX1::SioRxCh1 => {}
                EventX1::SoundCtcChannel0 => self.on_sound_ctc_zero(0, event.fire_cycle),
                EventX1::SoundCtcChannel1 => self.on_sound_ctc_zero(1, event.fire_cycle),
                EventX1::SoundCtcChannel2 => self.on_sound_ctc_zero(2, event.fire_cycle),
                EventX1::SoundCtcChannel3 => self.on_sound_ctc_zero(3, event.fire_cycle),
                EventX1::FmTimerA => self.on_fm_timer(0, event.fire_cycle),
                EventX1::FmTimerB => self.on_fm_timer(1, event.fire_cycle),
            }
        }
    }

    fn on_ctc_zero(&mut self, channel: usize, now: u64) {
        self.ctc.elapse(channel, now);
        // X1 CTC cascade wiring: channel 0's zero-count output clocks channel 3's
        // counter input, so software can divide the channel 0 timer down to a
        // lower-rate (e.g. frame) interrupt.
        if channel == 0 {
            self.ctc.trigger(3);
        }
        self.sync_interrupts();
        self.sync_ctc_schedule();
    }

    fn on_sound_ctc_zero(&mut self, channel: usize, now: u64) {
        self.sound_ctc.elapse(channel, now);
        self.sync_interrupts();
        self.sync_sound_ctc_schedule();
    }

    /// Handles an OPM timer overflow: it sets the chip's pollable status flag
    /// (the OPM IRQ is not wired to the CPU on this board) and reschedules.
    fn on_fm_timer(&mut self, timer_id: u32, now: u64) {
        if let Some(fm) = &mut self.fm {
            fm.timer_expired(timer_id, now);
        }
        self.apply_fm_timers();
    }

    /// Drains OPM timer schedule/cancel requests into the scheduler.
    fn apply_fm_timers(&mut self) {
        let mut actions: [Option<FmTimerAction>; 2] = [None, None];
        if let Some(fm) = &mut self.fm {
            for (slot, action) in actions.iter_mut().zip(fm.drain_timers().iter()) {
                *slot = Some(*action);
            }
        }
        for action in actions.into_iter().flatten() {
            match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => self
                    .scheduler
                    .schedule(EventX1::fm_timer(timer_id), fire_cycle),
                FmTimerAction::Cancel { timer_id } => {
                    self.scheduler.cancel(EventX1::fm_timer(timer_id))
                }
            }
        }
    }

    /// Reflects the keyboard, CTC and (on turbo) DMA and SIO interrupt lines into
    /// the daisy chain.
    fn sync_interrupts(&mut self) {
        self.interrupt
            .set(IrqSource::Keyboard, self.sub.key_irq_pending());
        self.interrupt.set(IrqSource::Ctc, self.ctc.has_pending());
        if self.model.has_fm() {
            self.interrupt
                .set(IrqSource::SoundCtc, self.sound_ctc.has_pending());
        }
        if self.model.is_turbo() {
            self.interrupt.set(IrqSource::Dma, self.dma.has_pending());
            self.interrupt.set(IrqSource::Sio, self.sio.has_pending());
        }
    }

    /// Schedules or cancels each main CTC channel's zero-count event.
    fn sync_ctc_schedule(&mut self) {
        for channel in 0..4 {
            let event = EventX1::ctc_channel(channel);
            match self.ctc.zero_cycle(channel) {
                Some(cycle) => self.scheduler.schedule(event, cycle),
                None => self.scheduler.cancel(event),
            }
        }
    }

    /// Schedules or cancels each sound-board CTC channel's zero-count event.
    fn sync_sound_ctc_schedule(&mut self) {
        for channel in 0..4 {
            let event = EventX1::sound_ctc_channel(channel);
            match self.sound_ctc.zero_cycle(channel) {
                Some(cycle) => self.scheduler.schedule(event, cycle),
                None => self.scheduler.cancel(event),
            }
        }
    }

    fn accumulate_rtc(&mut self, elapsed: u64) {
        self.rtc_accumulator += elapsed;
        let one_second = u64::from(self.clocks.main_clock_hz);
        while self.rtc_accumulator >= one_second {
            self.rtc_accumulator -= one_second;
            self.sub.tick_one_second();
        }
    }

    /// Advances the cassette waveform to the current cycle and reports the
    /// end-of-tape sensor to the sub-CPU.
    pub(crate) fn advance_cassette(&mut self) {
        self.cassette
            .advance(self.current_cycle, self.clocks.main_clock_hz);
        self.sub.set_tape_end(self.cassette.at_end());
    }

    /// Stops the deck when the tape runs against either end, reporting the
    /// stop to the sub-CPU as the deck's remote line dropping.
    fn auto_stop_cassette(&mut self) {
        if (self.cassette.is_moving_forward() && self.cassette.at_end())
            || (self.cassette.is_rewinding() && self.cassette.at_start())
        {
            self.cassette.stop();
            self.sub.notify_transport_stopped();
        }
    }

    /// Applies a transport action requested by the sub-CPU CMT state machine.
    fn apply_cassette_action(&mut self, action: CassetteAction) {
        self.advance_cassette();
        match action {
            CassetteAction::Play => self.cassette.play(),
            CassetteAction::Stop => self.cassette.stop(),
            CassetteAction::FastForward => self.cassette.fast_forward(),
            CassetteAction::Rewind => self.cassette.rewind(),
            CassetteAction::ApssForward => self.cassette.automatic_program_search(true),
            CassetteAction::ApssBackward => self.cassette.automatic_program_search(false),
            CassetteAction::Record => self.cassette.record(),
            CassetteAction::Eject => {
                self.cassette.eject();
                self.sub.set_tape_playable(false);
                self.sub.set_tape_end(false);
            }
        }
    }

    /// Parses a cassette image (chosen by file extension) and loads it into the
    /// deck, arming the sub-CPU tape-present sensor.
    pub fn insert_cassette(&mut self, extension: &str, image: &[u8]) -> Result<(), CassetteError> {
        let media = load_cassette(extension, image)?;
        self.cassette.insert_media(media);
        self.sub.set_tape_playable(true);
        self.sub.set_tape_end(false);
        Ok(())
    }

    /// Parses and loads a cassette image with its configured source path.
    pub fn insert_cassette_from_path(
        &mut self,
        extension: &str,
        image: &[u8],
        path: &std::path::Path,
    ) -> Result<(), CassetteError> {
        let media = load_cassette(extension, image)?;
        self.cassette.insert_media_from_path(media, path);
        self.sub.set_tape_playable(true);
        self.sub.set_tape_end(false);
        Ok(())
    }

    /// Removes the loaded cassette and clears the tape-present sensor.
    pub(crate) fn eject_cassette(&mut self) {
        self.cassette.eject();
        self.sub.set_tape_playable(false);
        self.sub.set_tape_end(false);
    }

    /// Sets and immediately seeds the calendar/clock from the host time provider.
    pub(crate) fn set_host_date_time_source(&mut self, source: SharedHostDateTimeSource) {
        let time = source.now();
        self.sub.set_host_time(
            time.year,
            time.month,
            time.day,
            time.day_of_week,
            time.hour,
            time.minute,
            time.second,
        );
    }

    /// Returns the number of frames presented within the current epoch.
    pub(crate) fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    /// Returns the fixed audio output rate in Hz.
    pub(crate) fn audio_sample_rate(&self) -> u32 {
        self.clocks.sample_rate
    }

    /// Returns the primary scheduling-tick rate as `(numerator, denominator)`.
    pub(crate) fn automation_timebase(&self) -> (u64, u64) {
        (u64::from(self.cpu_clock_hz()), 1)
    }

    /// Returns the stable automation model identifier.
    pub(crate) fn model_id(&self) -> &'static str {
        match self.model {
            X1Model::X1 => "x1",
            X1Model::X1Turbo => "x1turbo",
        }
    }

    /// Generates and discards audio covering `elapsed_ticks` for determinism.
    ///
    /// Sample counts use integer-remainder accounting so the generated sequence
    /// is identical across two identical runs.
    pub(crate) fn drain_automation_audio(&mut self, elapsed_ticks: u64) {
        let (numerator, denominator) = self.automation_timebase();
        let accumulator =
            elapsed_ticks as u128 * denominator as u128 * self.clocks.sample_rate as u128
                + self.automation_audio_remainder as u128;
        let frames = (accumulator / numerator as u128) as usize;
        self.automation_audio_remainder = (accumulator % numerator as u128) as u64;
        if frames == 0 {
            return;
        }
        let mut scratch = vec![0.0f32; frames * 2];
        let _ = self.generate_audio_samples(1.0, &mut scratch);
    }

    fn frame_period(&self) -> u64 {
        match self.frame_params.total_lines {
            Some(lines) => lines * self.horizontal_period_cycles(),
            None => u64::from(self.clocks.main_clock_hz) / REFRESH_HZ,
        }
    }

    fn total_lines(&self) -> u64 {
        self.frame_params
            .total_lines
            .unwrap_or_else(|| (self.frame_period() / self.horizontal_period_cycles()).max(1))
    }

    /// Re-latches the CRTC display parameters (start of frame, and immediately
    /// after the timing registers change).
    fn latch_frame_params(&mut self) {
        self.frame_params = latch_frame_params(&self.crtc, self.model, self.video.mode1());
    }

    /// The current vertical beam position (scanline within the frame).
    fn vertical_position(&self) -> u64 {
        let elapsed = self.current_cycle.saturating_sub(self.frame_start_cycle);
        (elapsed / self.horizontal_period_cycles()) % self.total_lines()
    }

    /// Assembles the PPI port B value from the sub-CPU handshake, the beam
    /// position, the RAM-bank flag and the break line.
    fn port_b(&self) -> u8 {
        let mut value = self.sub.port_b_handshake();

        let vpos = self.vertical_position();
        let char_height = u64::from(self.frame_params.ch_height);
        let vdisp_line = u64::from(self.frame_params.vt_disp) * char_height;
        let vsync_start = (u64::from(self.crtc.state.regs[7] & 0x7F) + 1) * char_height;
        let vsync_width = match u64::from(self.crtc.state.regs[3] >> 4) {
            0 => 16,
            width => width,
        };
        if vpos < vdisp_line {
            value |= PORT_B_VDISP;
        }
        if vpos >= vsync_start && vpos < vsync_start + vsync_width {
            value |= PORT_B_VSYNC;
        }
        if !self.memory.rom_selected() {
            value |= PORT_B_RAM_BANK;
        }
        if self.cassette.ear_level() {
            value |= PORT_B_CASSETTE;
        }
        if !self.sub.break_low() {
            value |= PORT_B_BREAK;
        }
        value
    }

    /// Computes the character code and glyph row under the CRT beam for the
    /// compatible PCG accesses. The beam position is reconstructed from the
    /// cycles elapsed since vertical blanking began: the line counter starts at
    /// the bottom of the displayed area and wraps through the frame, and the
    /// horizontal character position covers the full (blanked) line width.
    pub(crate) fn beam_code_line(&self) -> (u8, u8) {
        let params = &self.frame_params;
        let ch_height = u64::from(params.ch_height).max(1);
        let ht_clock = self.horizontal_period_cycles();
        let clock = self.current_cycle.saturating_sub(self.vblank_anchor_cycle);

        let vt_line = u64::from(params.vt_disp) * ch_height + clock / ht_clock;
        let mut address = (u64::from(params.hz_total) * (clock % ht_clock)) / ht_clock;
        address += u64::from(params.hz_disp) * (vt_line / ch_height);
        address &= 0x7FF;
        address = address.wrapping_add(u64::from(params.st_addr));

        let mut code = self.video.read_text(address as u16);
        let mut line = (vt_line % ch_height) as u8;
        match self.model {
            X1Model::X1 => {}
            X1Model::X1Turbo => {
                if self.video.read_kvram(address as u16) & 0x80 != 0 {
                    code = (code & 0xFE) | (line & 1);
                }
                if self.video.mode1() & (MODE1_CHAR_CLOCK_15 | MODE1_CG_STRIDE_400) != 0 {
                    line >>= 1;
                }
            }
        }
        (code, line & 7)
    }

    /// Latches the vertical-blanking anchor when the scanline event enters the
    /// blanked portion of the frame (the CRTC V-DISP falling edge).
    fn anchor_vblank_if_crossed(&mut self, fire_cycle: u64) {
        let horizontal_period = self.horizontal_period_cycles();
        let elapsed = fire_cycle.saturating_sub(self.frame_start_cycle);
        let line = (elapsed / horizontal_period) % self.total_lines();
        let first_blank_line =
            u64::from(self.frame_params.vt_disp) * u64::from(self.frame_params.ch_height);
        if line == first_blank_line {
            let line_start = fire_cycle - elapsed % horizontal_period;
            self.vblank_anchor_cycle = line_start;
        }
    }

    /// Watches CPU reads of the PPI port B: software that polls for vertical
    /// blanking observes the V-DISP bit drop here, and the beam clock for the
    /// PCG accesses is re-anchored to that exact moment.
    pub(crate) fn detect_vblank_poll(&mut self, port_b_value: u8) {
        let vdisp = port_b_value & PORT_B_VDISP != 0;
        if self.port_b_vdisp_seen && !vdisp {
            self.vblank_anchor_cycle = self.current_cycle;
        }
        self.port_b_vdisp_seen = vdisp;
    }

    fn latch_scanline(&mut self, fire_cycle: u64) {
        let horizontal_period = self.horizontal_period_cycles();
        let total_lines = self.total_lines();
        let elapsed = fire_cycle.saturating_sub(self.frame_start_cycle);
        let line = (elapsed / horizontal_period) % total_lines;
        if line >= u64::from(self.visible_scanlines() as u32) {
            return;
        }
        let inputs = RenderInputsX1 {
            text_vram: self.video.text_vram(),
            attr_vram: self.video.attr_vram(),
            kvram: self.video.kvram(),
            pcg: self.video.pcg(),
            gaiji: self.video.pcg_gaiji(),
            ank_rom: &self.ank_rom,
            kanji_rom: &self.kanji_rom,
            bitmap: self.video.bitmap(),
            palette: self.video.palette_guns(),
            priority: self.video.priority(),
            mode1: self.video.mode1(),
            mode2: self.video.mode2(),
            cblink: self.cblink,
            display_off: (self.crtc.state.regs[8] & 0x30) == 0x30,
            hz_disp: self.frame_params.hz_disp,
            vt_disp: self.frame_params.vt_disp,
            ch_height: self.frame_params.ch_height,
            st_addr: self.frame_params.st_addr,
            vt_ofs: self.frame_params.vt_ofs,
            hires: self.frame_params.hires,
            column40: self.column40,
            model: renderer_model(self.model),
        };
        self.renderer.latch_scanline(&inputs, line as usize);
    }

    fn present_latched_frame(&mut self) {
        let height = self.visible_scanlines();
        let (width, height) = self.renderer.present_latched_frame(height);
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
                Some(u64::from(self.cpu_clock_hz())),
            ),
            TraceEvent::Presentation(TracePresentation {
                display: trace_id::display::MAIN,
                frame: self.presented_frames,
                width: self.display_width,
                height: self.display_height,
            }),
        );
    }

    fn visible_scanlines(&self) -> usize {
        if self.frame_params.hires {
            DISPLAY_HEIGHT as usize * 2
        } else {
            DISPLAY_HEIGHT as usize
        }
    }

    fn horizontal_blank_start_cycles(&self) -> u64 {
        let horizontal_period = self.horizontal_period_cycles();
        let horizontal_total = u64::from(self.frame_params.hz_total).max(1);
        let horizontal_displayed = u64::from(self.frame_params.hz_disp);
        (horizontal_displayed * horizontal_period)
            .div_ceil(horizontal_total)
            .min(horizontal_period)
    }

    fn schedule_next_scanline_after(&mut self, cycle: u64) {
        let horizontal_period = self.horizontal_period_cycles();
        let blank_start = self.horizontal_blank_start_cycles();
        let elapsed = cycle.saturating_sub(self.frame_start_cycle);
        let line_start = self.frame_start_cycle + (elapsed / horizontal_period) * horizontal_period;
        let current_line_blank = line_start + blank_start;
        let fire_cycle = if current_line_blank > cycle {
            current_line_blank
        } else {
            line_start + horizontal_period + blank_start
        };
        self.scheduler.schedule(EventX1::Scanline, fire_cycle);
    }

    fn reset_display_timing(&mut self) {
        self.latch_frame_params();
        self.frame_start_cycle = self.current_cycle;
        self.renderer.clear_latched_frame();
        self.scheduler
            .schedule(EventX1::VBlank, self.current_cycle + self.frame_period());
        self.schedule_next_scanline_after(self.current_cycle);
    }

    /// Whether an interrupt is pending for the CPU.
    pub fn has_irq(&self) -> bool {
        self.interrupt.has_pending()
    }

    /// Dismisses the interrupt currently under service, as the CPU's `RETI`
    /// does. This re-enables lower-priority sources in the daisy chain and
    /// lets the serviced device drop a request it latched while under
    /// service, so a recurring source cannot re-fire back-to-back and starve
    /// the main program.
    pub fn notify_reti(&mut self) {
        match self.interrupt.end_service() {
            Some(IrqSource::Ctc) => {
                self.ctc.notify_reti();
                if self.ctc.has_in_service() {
                    // A nested channel handler returned; the device still
                    // holds the chain for the outer channel.
                    self.interrupt.begin_service(IrqSource::Ctc);
                }
                self.interrupt.set(IrqSource::Ctc, self.ctc.has_pending());
            }
            Some(IrqSource::SoundCtc) => {
                self.sound_ctc.notify_reti();
                if self.sound_ctc.has_in_service() {
                    self.interrupt.begin_service(IrqSource::SoundCtc);
                }
                self.interrupt
                    .set(IrqSource::SoundCtc, self.sound_ctc.has_pending());
            }
            Some(IrqSource::Dma) => {
                self.dma.notify_reti();
                self.interrupt.set(IrqSource::Dma, self.dma.has_pending());
            }
            Some(IrqSource::Keyboard) | Some(IrqSource::Sio) | None => {}
        }
    }

    /// Acknowledges an interrupt and returns its daisy-chain line and vector.
    pub fn acknowledge_irq(&mut self) -> (Option<u16>, u8) {
        let source = self.interrupt.highest_pending();
        if let Some(source) = source
            && source != IrqSource::Keyboard
        {
            // The keyboard/sub-CPU does not hold the chain under service: the
            // acknowledge itself dismisses its interrupt, and X1 key handlers
            // commonly return with RET rather than RETI.
            self.interrupt.begin_service(source);
        }
        let vector = match source {
            Some(IrqSource::Keyboard) => {
                self.interrupt.clear(IrqSource::Keyboard);
                self.sub.acknowledge_key_irq()
            }
            Some(IrqSource::SoundCtc) => {
                self.interrupt.clear(IrqSource::SoundCtc);
                let vector = self.sound_ctc.acknowledge();
                self.interrupt
                    .set(IrqSource::SoundCtc, self.sound_ctc.has_pending());
                vector
            }
            Some(IrqSource::Ctc) => {
                self.interrupt.clear(IrqSource::Ctc);
                let vector = self.ctc.acknowledge();
                self.interrupt.set(IrqSource::Ctc, self.ctc.has_pending());
                vector
            }
            Some(IrqSource::Dma) => {
                self.interrupt.clear(IrqSource::Dma);
                let vector = self.dma.acknowledge();
                self.interrupt.set(IrqSource::Dma, self.dma.has_pending());
                vector
            }
            Some(IrqSource::Sio) => {
                self.interrupt.clear(IrqSource::Sio);
                let vector = self.sio.acknowledge();
                self.interrupt.set(IrqSource::Sio, self.sio.has_pending());
                vector
            }
            None => 0,
        };
        (source.map(|source| source as u16), vector)
    }

    fn memory_read(&mut self, address: u16) -> u8 {
        self.memory.read(address)
    }

    fn memory_write(&mut self, address: u16, value: u8) {
        self.memory.write(address, value);
    }

    /// Charges the wait-state penalty for a bitmap VRAM access. The stall is the
    /// mode-dependent mean bus-contention wait, expressed as the exact fraction
    /// `sum / VRAM_WAIT_PERIOD`; the fractional part carries in
    /// `vram_wait_remainder` so consecutive accesses (including several in one
    /// instruction) average out to the mean without rounding bias.
    pub(crate) fn charge_vram_access_wait(&mut self) {
        let sum = match (self.column40, self.frame_params.hires) {
            (true, false) => io_wait::VRAM_WAIT_SUM_40,
            (false, false) => io_wait::VRAM_WAIT_SUM_80,
            (true, true) => io_wait::VRAM_WAIT_SUM_40_HIRES,
            (false, true) => io_wait::VRAM_WAIT_SUM_80_HIRES,
        };
        self.vram_wait_remainder += sum;
        let whole = self.vram_wait_remainder / io_wait::VRAM_WAIT_PERIOD;
        self.vram_wait_remainder -= whole * io_wait::VRAM_WAIT_PERIOD;
        self.add_wait_cycles(whole);
    }

    /// Adds wait-state cycles for the CPU to drain after the instruction.
    pub(crate) fn add_wait_cycles(&mut self, cycles: i64) {
        self.wait_cycles += cycles;
    }

    fn horizontal_period_cycles(&self) -> u64 {
        if self.frame_params.hires {
            HORIZONTAL_PERIOD_CYCLES_HIRES
        } else {
            HORIZONTAL_PERIOD_CYCLES
        }
    }
}

/// Latches the CRTC-derived display parameters for a frame. The hi-res scan is
/// selected on turbo models only, when the CRTC is programmed for more than 400
/// raster lines.
fn latch_frame_params(crtc: &Hd6845, model: X1Model, mode1: u8) -> FrameCrtcParams {
    let ch_height = crtc.char_height();
    let vt_disp = u16::from(crtc.state.regs[6] & 0x7F);
    let vertical_total = crtc.total_scanlines();
    let hires = match model {
        X1Model::X1 => false,
        X1Model::X1Turbo => u32::from(vertical_total) > HIRES_SCAN_LINE_THRESHOLD,
    };
    let displayed = vt_disp * ch_height;
    let total_lines =
        (displayed != 0 && vertical_total >= displayed).then_some(u64::from(vertical_total.max(1)));
    let kanji_underline = mode1 & MODE1_KANJI_UNDERLINE != 0;
    let mut vt_ofs = i32::from(crtc.state.regs[5] & 0x1F);
    match model {
        X1Model::X1 => vt_ofs -= 2,
        X1Model::X1Turbo => {
            if hires {
                if kanji_underline {
                    vt_ofs -= 8;
                }
            } else {
                vt_ofs -= 2;
                if kanji_underline {
                    vt_ofs -= 16;
                }
            }
        }
    }
    FrameCrtcParams {
        ch_height,
        hz_total: crtc.horizontal_total(),
        hz_disp: crtc.display_width_chars(),
        vt_disp,
        st_addr: crtc.start_address(),
        vt_ofs: vt_ofs.max(0) as u16,
        hires,
        total_lines,
    }
}

/// The renderer-side machine variant for `model`.
fn renderer_model(model: X1Model) -> X1RendererModel {
    match model {
        X1Model::X1 => X1RendererModel::Base,
        X1Model::X1Turbo => X1RendererModel::Turbo,
    }
}

/// Maps a joystick to an AY port byte (active low: pressed clears the bit).
fn joystick_to_port(state: JoystickState) -> u8 {
    let mut value = 0xFF;
    if state.up {
        value &= !0x01;
    }
    if state.down {
        value &= !0x02;
    }
    if state.left {
        value &= !0x04;
    }
    if state.right {
        value &= !0x08;
    }
    if state.trigger1 {
        value &= !0x20;
    }
    if state.trigger2 {
        value &= !0x40;
    }
    value
}

fn sub_poll_period(main_clock_hz: u32) -> u64 {
    (u64::from(main_clock_hz) * SUB_POLL_MICROS / 1_000_000).max(1)
}

/// Maps a kanji data-port address to an offset into the de-interleaved kanji ROM.
/// The mapping groups the JIS code ranges the BIOS uses for the kanji data ports;
/// unmapped codes read back as glyph zero.
fn jis_convert(kanji_address: u16) -> usize {
    let address = kanji_address as usize;
    let convert = |base: usize, low: usize, span: usize| -> usize {
        (base + (((address - low) & span) >> 3)) << 4
    };
    match address {
        0x0E00..=0x0E9F => convert(0x0E0, 0x0E00, 0x0FF),
        0x0F00..=0x109F => convert(0x4C0, 0x0F00, 0x1FF),
        0x1100..=0x129F => convert(0x2C0, 0x1100, 0x1FF),
        0x0100..=0x01FF => convert(0x040, 0x0100, 0x0FF),
        0x0500..=0x06FF => convert(0x240, 0x0500, 0x1FF),
        0x0300..=0x04FF => convert(0x440, 0x0300, 0x1FF),
        _ => 0,
    }
}

/// The kanji-port address of the first glyph of a JIS row. Returned (high byte)
/// by JIS-to-address conversion reads of port `0x0E80` when the latch high byte
/// is zero; the arithmetic wraps for rows outside the mapped ranges.
fn jis_row_address(jis_row: u8) -> u16 {
    let row = u16::from(jis_row);
    if row > 0x28 {
        0x4000u16.wrapping_add(row.wrapping_sub(0x30).wrapping_mul(0x600))
    } else {
        0x0100u16.wrapping_add(row.wrapping_sub(0x21).wrapping_mul(0x600))
    }
}

/// Ephemeral `common::Bus` adapter for the main Z80.
pub struct MainBusView<'a, T: TraceSink = NoTrace> {
    pub bus: &'a mut X1Bus<T>,
}

impl<T: TraceSink> common::Bus for MainBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let bus_address = address as u16;
        let value = self.bus.memory_read(bus_address);
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
        let bus_address = address as u16;
        self.bus.memory_write(bus_address, value);
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

    fn io_read_byte(&mut self, port: u16) -> u8 {
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

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        match self.bus.model {
            X1Model::X1 => {
                if self.bus.memory.rom_selected() {
                    self.bus.wait_cycles += M1_ROM_FETCH_WAIT_CYCLES;
                }
            }
            X1Model::X1Turbo => {}
        }
        let value = self.bus.memory_read(address as u16);
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

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.bus.wait_cycles)
    }

    fn has_irq(&self) -> bool {
        self.bus.has_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let (line, vector) = self.bus.acknowledge_irq();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::X1_DAISY,
                    TraceInterruptKind::Maskable,
                    line,
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

    fn notify_reti(&mut self) {
        self.bus.notify_reti();
    }

    fn on_instruction_end(&mut self) {
        self.bus.do_dma();
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

#[cfg(test)]
mod tests {
    use common::Bus as _;

    use super::*;

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

    fn make_bus(model: X1Model) -> X1Bus {
        let mut bus = X1Bus::<NoTrace>::new(model, 48_000);
        bus.load_roms(&crate::rom::LoadedRoms {
            model,
            ipl: vec![0xC3; 0x1000],
            cgrom_8x8: Vec::new(),
            ank: Vec::new(),
            kanji: None,
        });
        bus
    }

    #[test]
    fn traces_use_live_io_decode_and_daisy_source() {
        let mut bus = X1Bus::new_with_trace_sink(X1Model::X1, 48_000, RecordingTrace::default());
        {
            let mut view = MainBusView { bus: &mut bus };
            common::Bus::io_read_byte(&mut view, 0x1A00);
            common::Bus::io_read_byte(&mut view, 0x0800);
        }
        bus.interrupt.raise(IrqSource::Keyboard);
        {
            let mut view = MainBusView { bus: &mut bus };
            common::Bus::acknowledge_irq(&mut view);
        }

        assert!(bus.tracer().accesses[0].handled);
        assert!(!bus.tracer().accesses[1].handled);
        assert_eq!(
            bus.tracer().interrupts[0].line,
            Some(IrqSource::Keyboard as u16)
        );
    }

    #[test]
    fn memory_trace_addresses_apply_the_z80_address_mask() {
        let mut bus = X1Bus::new_with_trace_sink(X1Model::X1, 48_000, RecordingTrace::default());
        {
            let mut view = MainBusView { bus: &mut bus };
            common::Bus::read_byte(&mut view, 0x1_1234);
            common::Bus::write_byte(&mut view, 0x1_5678, 0xA5);
            common::Bus::fetch_opcode_byte(&mut view, 0x1_9ABC);
        }

        assert_eq!(bus.tracer().accesses[0].address, 0x1234);
        assert_eq!(bus.tracer().accesses[1].address, 0x5678);
        assert_eq!(bus.tracer().accesses[2].address, 0x9ABC);
    }

    #[test]
    fn rom_ram_toggle_switches_the_bottom_half() {
        let mut bus = make_bus(X1Model::X1);
        assert!(bus.rom_selected());
        assert_eq!(bus.peek_byte(0x0000), 0xC3);

        bus.io_write(0x1E00, 0x00);
        assert!(!bus.rom_selected());
        assert_eq!(bus.peek_byte(0x0000), 0x00);

        bus.io_write(0x1D00, 0x00);
        assert!(bus.rom_selected());
        assert_eq!(bus.peek_byte(0x0000), 0xC3);
    }

    #[test]
    fn dip_switch_reports_the_selected_monitor() {
        let mut bus = make_bus(X1Model::X1Turbo);
        // Default (Auto) and Fixed24kHz report a high-resolution monitor: bit 0 = 0.
        assert_eq!(bus.io_read(0x1FF0).0 & 0x01, 0x00);
        bus.set_monitor_timing(MonitorTiming::Fixed24kHz);
        assert_eq!(bus.io_read(0x1FF0).0 & 0x01, 0x00);
        // A standard 15 kHz monitor sets bit 0.
        bus.set_monitor_timing(MonitorTiming::Fixed15kHz);
        assert_eq!(bus.io_read(0x1FF0).0 & 0x01, 0x01);
    }

    #[test]
    fn bitmap_vram_reads_and_writes_accumulate_the_mean_wait() {
        let mut bus = make_bus(X1Model::X1);
        let period = io_wait::VRAM_WAIT_PERIOD;
        let sum = io_wait::VRAM_WAIT_SUM_80;
        // 80-column mean is ~1.78: the fractional carry makes the first access
        // charge floor(sum / period) = 1 and the second floor(2 * sum / period) - 1 = 2.
        let first = sum / period;
        let second = (2 * sum) / period - first;

        bus.io_read(0x4000);
        bus.io_write(0x8000, 0x55);
        assert_eq!(bus.wait_cycles, first + second);

        let mut view = MainBusView { bus: &mut bus };
        assert_eq!(view.drain_wait_cycles(), first + second);
        assert_eq!(view.drain_wait_cycles(), 0);
    }

    #[test]
    fn column40_selects_the_40_column_mean() {
        let mut bus = make_bus(X1Model::X1);
        bus.column40 = true;
        bus.charge_vram_access_wait();
        assert_eq!(
            bus.wait_cycles,
            io_wait::VRAM_WAIT_SUM_40 / io_wait::VRAM_WAIT_PERIOD
        );
    }

    #[test]
    fn multi_plane_latch_writes_below_the_window_take_the_vram_wait() {
        let mut bus = make_bus(X1Model::X1);
        bus.io_write(0x2000, 0x00);
        assert_eq!(bus.wait_cycles, 0);

        bus.video.latch_vram_mode();
        bus.io_write(0x2000, 0x00);
        assert_eq!(
            bus.wait_cycles,
            io_wait::VRAM_WAIT_SUM_80 / io_wait::VRAM_WAIT_PERIOD
        );
    }

    #[test]
    fn mailbox_and_psg_ports_take_one_wait_cycle() {
        let mut bus = make_bus(X1Model::X1);
        for port in [0x1900u16, 0x1B00, 0x1C00] {
            bus.io_read(port);
            assert_eq!(bus.wait_cycles, 1, "read {port:#06X}");
            bus.wait_cycles = 0;
            bus.io_write(port, 0x00);
            assert_eq!(bus.wait_cycles, 1, "write {port:#06X}");
            bus.wait_cycles = 0;
        }
        for port in [0x1800u16, 0x1A00, 0x3000] {
            bus.io_read(port);
            bus.io_write(port, 0x00);
            assert_eq!(bus.wait_cycles, 0, "{port:#06X}");
        }
    }

    #[test]
    fn crtc_decode_mirrors_every_16_ports() {
        let mut bus = make_bus(X1Model::X1Turbo);
        bus.io_write(0x1800, 1);
        bus.io_write(0x1801, 80);
        bus.io_write(0x1802, 1);
        bus.io_write(0x1803, 40);
        assert_eq!(bus.crtc.display_width_chars(), 80);

        bus.io_write(0x1810, 1);
        bus.io_write(0x18F1, 40);
        assert_eq!(bus.crtc.display_width_chars(), 40);
    }

    #[test]
    fn write_only_display_registers_read_open_bus() {
        let mut bus = make_bus(X1Model::X1Turbo);
        bus.io_write(0x1000, 0xAA);
        bus.io_write(0x1100, 0xCC);
        bus.io_write(0x1200, 0xF0);
        bus.io_write(0x1300, 0x55);
        bus.io_write(0x1FD0, 0x35);
        bus.io_write(0x1FE0, 0x1F);
        for port in [0x1000u16, 0x1100, 0x1200, 0x1300, 0x1FD0, 0x1FE0] {
            assert_eq!(bus.io_read(port).0, OPEN_BUS, "{port:#06X}");
        }
    }

    #[test]
    fn mode_registers_decode_only_their_exact_ports() {
        let mut bus = make_bus(X1Model::X1Turbo);
        bus.io_write(0x1FD1, 0x10);
        bus.io_write(0x1FE1, 0x08);
        assert_eq!(bus.video.mode1(), 0);
        assert_eq!(bus.video.mode2(), 0);

        bus.io_write(0x1FD0, 0x10);
        bus.io_write(0x1FE0, 0x08);
        assert_eq!(bus.video.mode1(), 0x10);
        assert_eq!(bus.video.mode2(), 0x08);
    }

    #[test]
    fn kanji_data_ports_read_glyph_rows_with_row_advance() {
        let mut bus = make_bus(X1Model::X1Turbo);
        let mut rom = vec![0u8; 0x2_0000];
        // Latch address 0x0E00 maps to glyph base 0x0E00 in the ROM.
        for row in 0..16usize {
            rom[0x0E00 + row] = 0x10 + row as u8;
            rom[0x0E00 + 16 + row] = 0x30 + row as u8;
        }
        bus.kanji_rom = rom;

        bus.io_write(0x0E80, 0x00);
        bus.io_write(0x0E81, 0x0E);
        bus.io_write(0x0E82, 0x01);
        for row in 0..16u8 {
            assert_eq!(bus.io_read(0x0E80).0, 0x10 + row);
            // The row only advances after both halves were read.
            assert_eq!(bus.io_read(0x0E80).0, 0x10 + row);
            assert_eq!(bus.io_read(0x0E81).0, 0x30 + row);
        }
        // After sixteen rows the counter wraps to the first row.
        assert_eq!(bus.io_read(0x0E80).0, 0x10);
    }

    #[test]
    fn kanji_data_port_converts_jis_row_to_address_when_high_byte_clear() {
        let mut bus = make_bus(X1Model::X1Turbo);
        bus.kanji_rom = vec![0u8; 0x2_0000];
        bus.io_write(0x0E80, 0x21);
        bus.io_write(0x0E81, 0x00);
        assert_eq!(bus.io_read(0x0E80).0, 0x01);
        assert_eq!(bus.io_read(0x0E81).0, 0x00);

        bus.io_write(0x0E80, 0x30);
        assert_eq!(bus.io_read(0x0E80).0, 0x40);
    }

    /// Programs a 100-column (80 displayed) by 25-row, 8-line, 256-line frame.
    fn program_standard_crtc(bus: &mut X1Bus) {
        for (register, value) in [(0u8, 99u8), (1, 80), (4, 31), (5, 0), (6, 25), (9, 7)] {
            bus.io_write(0x1800, register);
            bus.io_write(0x1801, value);
        }
    }

    #[test]
    fn beam_pcg_write_addresses_the_cell_under_the_beam() {
        let mut bus = make_bus(X1Model::X1);
        program_standard_crtc(&mut bus);
        let cg = [0u8; 0x800];

        // At the vertical-blanking anchor the beam sits at the start of the
        // first blanked row: text cell 80 * 25 = 2000.
        bus.io_write(0x3000 | (2000 & 0x7FF), 0x42);
        bus.set_current_cycle(0);
        bus.io_write(0x1500, 0xAA);
        assert_eq!(bus.video.read_pcg(0x42, 0, 1, &cg), 0xAA);

        // Half a scanline later the horizontal beam has crossed 50 of the 100
        // character columns: cell (2000 + 50) & 0x7FF = 2.
        bus.io_write(0x3002, 0x33);
        bus.set_current_cycle(125);
        bus.io_write(0x1500, 0xBB);
        assert_eq!(bus.video.read_pcg(0x33, 0, 1, &cg), 0xBB);

        // One scanline into blanking the beam is on the same character row, one
        // glyph row further down.
        bus.set_current_cycle(250);
        bus.io_write(0x1500, 0xCC);
        assert_eq!(bus.video.read_pcg(0x42, 1, 1, &cg), 0xCC);
    }

    #[test]
    fn vsync_pulse_has_a_start_and_a_width() {
        let mut bus = make_bus(X1Model::X1);
        program_standard_crtc(&mut bus);
        bus.io_write(0x1800, 3);
        bus.io_write(0x1801, 0x24); // vsync width 2 rasters
        bus.io_write(0x1800, 7);
        bus.io_write(0x1801, 28); // vsync from row 29

        bus.set_current_cycle(231 * 250);
        assert_eq!(bus.port_b() & PORT_B_VSYNC, 0);
        bus.set_current_cycle(232 * 250);
        assert_ne!(bus.port_b() & PORT_B_VSYNC, 0);
        bus.set_current_cycle(233 * 250);
        assert_ne!(bus.port_b() & PORT_B_VSYNC, 0);
        bus.set_current_cycle(234 * 250);
        assert_eq!(bus.port_b() & PORT_B_VSYNC, 0);
    }

    #[test]
    fn port_b_poll_realigns_the_vblank_anchor() {
        let mut bus = make_bus(X1Model::X1);
        program_standard_crtc(&mut bus);

        bus.set_current_cycle(199 * 250);
        bus.io_read(0x1A01); // V-DISP still high
        assert_eq!(bus.vblank_anchor_cycle, 0);

        bus.set_current_cycle(200 * 250 + 40);
        bus.io_read(0x1A01); // V-DISP observed dropping: re-anchor here
        assert_eq!(bus.vblank_anchor_cycle, 200 * 250 + 40);
    }

    #[test]
    fn pcg_direct_font_read_selects_the_ank16_and_cg_rom_fonts() {
        let mut bus = make_bus(X1Model::X1Turbo);
        bus.ank_rom = vec![0u8; 0x2000];
        bus.ank_rom[ANK16_ROM_OFFSET + 0x41 * 16 + 5] = 0x5A;
        bus.cg_rom = vec![0u8; 0x800];
        bus.cg_rom[0x41 * 8 + 2] = 0xA5;
        // The staged cell is the first magic cell without the PCG-select bit.
        bus.video.write_text(0x7FF, 0x41);

        bus.io_write(0x1FD0, 0x60); // direct access + 8x16 ANK font
        assert_eq!(bus.io_read(0x1405).0, 0x5A);

        bus.io_write(0x1FD0, 0x20); // direct access, 8x8 CG-ROM font
        assert_eq!(bus.io_read(0x1405).0, 0xA5);
    }

    #[test]
    fn base_x1_m1_fetch_waits_only_while_rom_is_mapped() {
        let mut bus = make_bus(X1Model::X1);
        {
            let mut view = MainBusView { bus: &mut bus };
            let _ = view.fetch_opcode_byte(0x0000);
            assert_eq!(view.drain_wait_cycles(), M1_ROM_FETCH_WAIT_CYCLES);
        }

        bus.io_write(0x1E00, 0x00);
        {
            let mut view = MainBusView { bus: &mut bus };
            let _ = view.fetch_opcode_byte(0x0000);
            assert_eq!(view.drain_wait_cycles(), 0);
        }

        let mut turbo = make_bus(X1Model::X1Turbo);
        let mut view = MainBusView { bus: &mut turbo };
        let _ = view.fetch_opcode_byte(0x0000);
        assert_eq!(view.drain_wait_cycles(), 0);
    }

    #[test]
    fn hires_port_b_uses_the_24khz_horizontal_period() {
        let mut bus = make_bus(X1Model::X1Turbo);
        for (register, value) in [(4u8, 27u8), (5, 0), (6, 25), (9, 15)] {
            bus.io_write(0x1800, register);
            bus.io_write(0x1801, value);
        }
        assert_eq!(bus.frame_period(), 448 * HORIZONTAL_PERIOD_CYCLES_HIRES);

        bus.set_current_cycle(HORIZONTAL_PERIOD_CYCLES_HIRES * 399);
        assert_ne!(bus.port_b() & PORT_B_VDISP, 0);

        bus.set_current_cycle(HORIZONTAL_PERIOD_CYCLES_HIRES * 400);
        assert_eq!(bus.port_b() & PORT_B_VDISP, 0);
    }

    #[test]
    fn turbo_crtc_programming_switches_to_the_hires_mean() {
        // Charges three VRAM accesses from a cleared carry; the accumulated wait
        // is floor(3 * mean), which distinguishes the 80-column normal mean
        // (~1.78) from the hi-res mean (~1.01).
        fn charge_three(bus: &mut X1Bus) -> i64 {
            bus.wait_cycles = 0;
            bus.vram_wait_remainder = 0;
            for _ in 0..3 {
                bus.charge_vram_access_wait();
            }
            bus.wait_cycles
        }
        let period = io_wait::VRAM_WAIT_PERIOD;

        let mut bus = make_bus(X1Model::X1Turbo);
        assert_eq!(
            charge_three(&mut bus),
            3 * io_wait::VRAM_WAIT_SUM_80 / period
        );

        // 31 character rows of 16 raster lines each: a 496-line vertical total,
        // which selects the 24 kHz hi-res scan.
        for (register, value) in [(4u8, 30u8), (5, 0), (9, 15)] {
            bus.io_write(0x1800, register);
            bus.io_write(0x1801, value);
        }
        assert_eq!(
            charge_three(&mut bus),
            3 * io_wait::VRAM_WAIT_SUM_80_HIRES / period
        );

        // The base X1 never uses the hi-res scan.
        let mut base = make_bus(X1Model::X1);
        for (register, value) in [(4u8, 30u8), (5, 0), (9, 15)] {
            base.io_write(0x1800, register);
            base.io_write(0x1801, value);
        }
        assert_eq!(
            charge_three(&mut base),
            3 * io_wait::VRAM_WAIT_SUM_80 / period
        );
    }
}
