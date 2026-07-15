//! PC-8801 system bus.
//!
//! Holds the shared machine state (main-CPU memory, the bank-control registers,
//! the scheduler, the cycle clock, the text-display devices, the kanji ROMs, and
//! the disk sub-system) and exposes it to the two Z80 cores through ephemeral
//! [`MainBusView`] / [`SubBusView`] adapters that implement `common::Bus`.

mod init;
mod ppi_link;
mod sub_fdc;
mod sub_io_read;
mod sub_io_write;
mod sub_mem;

use common::{
    HostDateTimeProvider, JoystickState, MonitorTiming, NoTrace, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceContext, TraceEvent, TraceInterruptAction, TracePresentation,
    TraceSink, trace_id,
};
use device::{
    beeper::Beeper,
    cdrom::CdImage,
    cdrom_pc88::Pc88Cdrom,
    i8214_pic::{I8214Pic, LEVEL_CLOCK, LEVEL_INT4, LEVEL_RXRDY, LEVEL_VRTC},
    i8251_serial::I8251Serial,
    i8255::I8255,
    i8257_dma::{I8257Dma, TEXT_CHANNEL},
    opn_fm::FmTimerAction,
    palette_pc88::{BACKGROUND_PEN, Pc88Palette},
    soundboard_ii::SoundboardII,
    upd765a_fdc::{FloppyController, UPD765_PLATFORM_STANDARD, Upd765aFdc},
    upd3301_crtc::{STATUS_DISPLAY_ENABLE, STATUS_UNDERRUN, Upd3301},
    upd4990a_rtc::Upd4990aRtc,
};
use software_renderer::{
    GraphicsMode88, Pc88Renderer, RenderInputs88,
    pc88::{PC88_MAX_HEIGHT, PC88_WIDTH},
};
use sub_mem::SubMemory;

use crate::{
    config::{BootMode, ClockConfig, EightMhzWaitMode, MemoryWaitSwitch, Pc8801Model},
    memory::{GvramSelect, Pc8801Memory, Pc8801MemoryTarget},
    scheduler::{Event88, Pc8801Scheduler},
};

/// Open-bus value returned for unimplemented I/O reads.
const OPEN_BUS: u8 = 0xFF;

/// DMA channel that feeds the CD-ROM SCSI data transfer (DREQ1 on the MC).
const CDROM_DMA_CHANNEL: usize = 1;

/// Port 0x40 read bit 5: set while the display is in vertical retrace (VRTC).
const PORT40_VRTC: u8 = 0x20;
/// Port 0x40 read bit 3: BOOT. The N88 reset code samples this DIP-switch line to
/// choose its boot device: when set it goes straight to the N88-BASIC ROM, when
/// clear it boots the disk system from drive 0/1. The real machine wires this to a
/// fixed switch; we drive it from disk presence instead.
const PORT40_BOOT_ROM: u8 = 0x08;
/// Port 0x40 read base value with the active-high idle bits set: bit 0 printer
/// not busy, bit 1 normal-resolution monitor, bit 2 serial DCD idle, bits 6-7
/// unused-high. Refined once the RTC line is wired.
const PORT40_READ_BASE: u8 = 0b1100_0111;

/// CLOCK interrupt frequency in Hz (the standalone 600 Hz system timer).
const CLOCK_TIMER_HZ: u64 = 600;

/// Horizontal scan frequency in Hz for the 15 kHz (200-line) monitor.
///
/// Calibration item: the per-line period is derived from this and the main CPU
/// clock; the resulting frame rate (about 62.4 Hz at 25 lines) is checked against
/// the ROM-measured VRTC cadence.
const HORIZONTAL_FREQ_15KHZ: u64 = 15_980;
/// Horizontal scan frequency in Hz for the 24 kHz (400-line) monitor.
const HORIZONTAL_FREQ_24KHZ: u64 = 24_823;

/// Port 0x32 PMODE bit: palette analog (two writes per pen) versus digital.
const MISC_CTRL_PMODE: u8 = 0x20;
/// Port 0x32 GVAM bit: enables the extended GVRAM/ALU access decode.
const MISC_CTRL_GVAM: u8 = 0x40;
/// Port 0x32 SINTM bit (active high): when set, the OPNA IRQ is masked off INT4.
const MISC_CTRL_SINTM: u8 = 0x80;
/// Port 0x33 N80 bit: masks the PSG/OPN interrupt.
const N80_CTRL_SINTM: u8 = 0x02;
/// Port 0x33 N80 bit: enables extended GVRAM/ALU access.
const N80_CTRL_GVAM: u8 = 0x40;
/// Port 0x33 N80 bit: selects the PC-8001mkIISR N80SR ROM personality.
const N80_CTRL_N80SR: u8 = 0x80;
/// Port 0x33 N80 readable/writable bits currently modeled.
const N80_CTRL_READ_MASK: u8 = N80_CTRL_SINTM | 0x04 | 0x08 | 0x10 | N80_CTRL_GVAM | N80_CTRL_N80SR;
/// Port 0x40 write bit 5: beeper enable.
const PORT40_BEEP_ENABLE: u8 = 0x20;
/// Port 0x40 write bit 1: RTC strobe.
const PORT40_RTC_STB: u8 = 0x02;
/// Port 0x40 write bit 2: RTC clock.
const PORT40_RTC_CLK: u8 = 0x04;
/// Port 0x40 read bit 4: RTC serial data out.
const PORT40_RTC_DOUT: u8 = 0x10;
/// Port 0x10 write bits 0-2: RTC command lines C0/C1/C2.
const PORT10_RTC_COMMAND: u8 = 0x07;
/// Port 0x10 write bit 3: RTC serial data in.
const PORT10_RTC_DIN: u8 = 0x08;
/// uPD4990A command-byte bit 3: strobe.
const RTC_CHIP_STB: u8 = 0x08;
/// uPD4990A command-byte bit 4: clock.
const RTC_CHIP_CLK: u8 = 0x10;
/// uPD4990A command-byte bit 5: serial data in.
const RTC_CHIP_DIN: u8 = 0x20;
/// Port 0x53 bit 0: text layer display disable.
const LAYER_DISABLE_TEXT: u8 = 0x01;
/// Port 0x30 write bit 0: 40-column when clear.
const PORT30_WIDTH40: u8 = 0x01;
/// Port 0x30 write bit 1: monochrome text when set.
const PORT30_COLOR: u8 = 0x02;
/// Port 0x31 (gfx_ctrl) mask selecting 400-line text.
const GFX_CTRL_400LINE_MASK: u8 = 0x11;
/// Port 0x31 (gfx_ctrl) bit 3: graphics layer enable.
const GFX_CTRL_GRPHE: u8 = 0x08;
/// Port 0x31 (gfx_ctrl) bit 4: 8-color graphics versus attribute color.
const GFX_CTRL_HCOLOR: u8 = 0x10;
/// Port 0x40 read/write bit 4: high-speed GVRAM access.
const PORT40_GHSM: u8 = 0x10;
/// Port 0x40 write bit 6: mouse latch strobe (routed to the OPN port A line).
const PORT40_MOUSE_STROBE: u8 = 0x40;
/// Mouse button line bit 4 (left button), active low on the SSG read port.
const MOUSE_BUTTON_LEFT: u8 = 0x10;
/// Mouse button line bit 5 (right button), active low on the SSG read port.
const MOUSE_BUTTON_RIGHT: u8 = 0x20;
/// Strobe-gap timeout for the mouse readout, in milliseconds: a longer gap
/// restarts the nibble sequence at the X high nibble.
const MOUSE_STROBE_TIMEOUT_MS: u64 = 3;
/// Joystick direction bits on OPN port A (register 0x0E), active low.
const JOYSTICK_UP: u8 = 0x01;
const JOYSTICK_DOWN: u8 = 0x02;
const JOYSTICK_LEFT: u8 = 0x04;
const JOYSTICK_RIGHT: u8 = 0x08;
/// Joystick trigger bits on OPN port B (register 0x0F), active low.
const JOYSTICK_TRIGGER1: u8 = 0x01;
const JOYSTICK_TRIGGER2: u8 = 0x02;
/// Size in bytes of one GVRAM plane (16 KiB; planes are blue, red, green).
const GVRAM_PLANE_SIZE: usize = 0x4000;
/// First CPU address of the text-VRAM-capable high region (0xF000-0xFFFF), used
/// by the V1H/V2 M1 fetch wait.
const HIGH_REGION_M1_START: u16 = 0xF000;
/// Main clock threshold separating the 4 MHz and 8 MHz CPU speeds.
const CPU_CLOCK_LOW_THRESHOLD_HZ: u32 = 6_000_000;
/// Per-access GVRAM accumulator increment, added on each GVRAM access.
const GVRAM_ACCESS_INCREMENT: i64 = 0x100;
/// GVRAM accumulator limit at 4 MHz (reads and writes share the same limit).
const GVRAM_ACCESS_LIMIT_4MHZ: i64 = 0x1B00;
/// GVRAM accumulator read limit at 8 MHz.
const GVRAM_ACCESS_LIMIT_8MHZ_READ: i64 = 0x02B0;
/// GVRAM accumulator write limit at 8 MHz.
const GVRAM_ACCESS_LIMIT_8MHZ_WRITE: i64 = 0x029C;

/// Slice clamp (main-clock units) while a PPI handshake is in flight: about one
/// sub-CPU instruction.
pub(crate) const SYNC_SLICE: u64 = 4;
/// Default fine interleave slice (main-clock units) when the link is idle.
pub(crate) const TIGHT_SLICE: u64 = 16;

/// Default host local-time source: returns the current system time as the
/// 6-byte BCD buffer the uPD4990A expects:
/// `[year, month<<4|day_of_week, day, hour, minute, second]`.
/// PC-8801 system bus shared by the main and sub CPUs.
pub struct Pc8801Bus<T: TraceSink = NoTrace> {
    pub(crate) memory: Pc8801Memory,
    pub(crate) scheduler: Pc8801Scheduler,
    /// Main-CPU priority interrupt controller (i8214), ports 0xE4/0xE6.
    pub(crate) pic: I8214Pic,
    /// uPD3301 text CRTC (ports 0x50/0x51).
    pub(crate) crtc: Upd3301,
    /// uPD8257 text DMA controller (ports 0x60-0x68).
    pub(crate) dma: I8257Dma,
    /// PC-88 graphics/background palette (ports 0x52/0x54-0x5B).
    pub(crate) palette: Pc88Palette,
    /// PC-88 text renderer.
    pub(crate) renderer: Pc88Renderer,
    /// Internal YM2608 (OPNA), "Sound Board II", at main I/O 0x44-0x47.
    pub(crate) soundboard_ii: SoundboardII,
    /// PC-8801-31 CD-ROM interface and PC-8801-30 SCSI drive (ports 0x90-0x9F).
    pub(crate) cdrom: Pc88Cdrom,
    /// Fixed-tone 1-bit beeper, gated by port 0x40 bit 5.
    pub(crate) beeper: Beeper,
    /// uPD4990A real-time clock. C0/C1/C2/DIN arrive on port 0x10, STB/CLK on
    /// port 0x40, and DOUT is read back from port 0x40 bit 4.
    pub(crate) rtc: Upd4990aRtc,
    /// i8251 USART (RS-232C, ports 0x20/0x21), modeled as no-cable.
    pub(crate) serial: I8251Serial,
    /// Host BCD local-time source used by the RTC's TIME_READ command.
    pub(crate) host_date_time_provider: HostDateTimeProvider,
    /// Port 0x10 write latch (RTC command/data lines and printer data).
    pub(crate) port10: u8,
    /// Level-1 kanji ROM read-window address latch (ports 0xE8/0xE9).
    pub(crate) kanji1_addr: u16,
    /// Level-2 kanji ROM read-window address latch (ports 0xEC/0xED).
    pub(crate) kanji2_addr: u16,
    pub(crate) current_cycle: u64,
    pub(crate) next_event_cycle: u64,
    /// Whether the display is currently in vertical retrace (port 0x40 bit 5).
    pub(crate) vrtc_active: bool,
    /// CLOCK timer period in main-CPU cycles.
    pub(crate) clock_timer_period: u64,
    /// Per-scanline period in main-CPU cycles, derived from the monitor frequency.
    pub(crate) crtc_line_period: u64,
    /// Character row currently being fed by the text DMA within the frame.
    pub(crate) crtc_current_row: u32,
    /// 24 kHz (400-line capable) monitor timing. False for the 15 kHz default.
    pub(crate) hireso: bool,
    /// Port 0x30 write latch (text width / color / CMT select).
    pub(crate) port30: u8,
    /// Port 0x40 write latch. Bit 4 is GHSM (high-speed GVRAM access).
    pub(crate) port40: u8,
    /// Port 0x6F baud-rate latch. The MA defaults to 1200 bps (selector 4).
    pub(crate) baud_rate: u8,
    /// Port 0x53 active-high layer disable flags.
    pub(crate) layer_disable: u8,
    /// Active-low keyboard matrix rows read at ports 0x00-0x0F.
    pub(crate) keyboard_rows: [u8; 16],
    /// Accumulated mouse movement since power-on (host deltas summed in).
    pub(crate) mouse_x: i32,
    pub(crate) mouse_y: i32,
    /// Accumulated movement captured at the last latch, used to compute the
    /// reported delta on the next latch.
    pub(crate) mouse_latch_x: i32,
    pub(crate) mouse_latch_y: i32,
    /// Latched 16-bit delta shifted out a nibble at a time (X in the high byte,
    /// Y in the low byte).
    pub(crate) mouse_data: u16,
    /// Mouse readout phase (0-3), advanced by each strobe edge; the X high
    /// nibble is presented at phase 0 through to the Y low nibble at phase 3.
    pub(crate) mouse_phase: u8,
    /// Previous strobe level (port 0x40 bit 6) for edge detection.
    pub(crate) mouse_strobe_level: bool,
    /// Cycle of the last strobe edge; a longer gap restarts the sequence.
    pub(crate) mouse_strobe_cycle: u64,
    /// Mouse button lines (bit 4 left, bit 5 right), active low.
    pub(crate) mouse_buttons: u8,
    /// Strobe-gap timeout in main-clock cycles.
    pub(crate) mouse_timeout_cycles: u64,
    /// Whether a mouse strobe edge has ever been seen. Until then the joystick
    /// owns the shared OPN port A line.
    pub(crate) mouse_strobe_seen: bool,
    /// Joystick direction lines for the OPN port A read (registers 0x0E),
    /// active low: bit0 up, bit1 down, bit2 left, bit3 right; bits 7:4 = 1.
    pub(crate) joystick_port_a: u8,
    /// Joystick trigger lines for the OPN port B read (register 0x0F),
    /// active low: bit0 trigger1, bit1 trigger2; bits 7:2 = 1.
    pub(crate) joystick_port_b: u8,
    /// Monitor timing selection (15 kHz, 24 kHz, or software-derived).
    pub(crate) monitor_timing: MonitorTiming,
    /// CPU clock cycles the text DMA holds the bus per character row in V1S/N
    /// modes (the BUSREQ lockout window), recomputed each frame.
    pub(crate) busreq_clocks: u64,
    /// Cycle until which the main CPU is locked off the bus by the text DMA. The
    /// CPU does not execute while `current_cycle < busreq_until`.
    pub(crate) busreq_until: u64,
    /// Accumulated memory access-wait cycles, drained by the CPU after each
    /// instruction (see `MainBusView::drain_wait_cycles`).
    pub(crate) memory_wait_cycles: i64,
    /// Whether the main CPU runs at the low (4 MHz) clock speed.
    pub(crate) cpu_clock_low: bool,
    /// Whether the optional memory wait states are inserted (memory wait switch
    /// set to compatible).
    pub(crate) mem_wait_on: bool,
    /// Whether the 8 MHz high-speed wait mode is active (omits the extra 8 MHz
    /// wait state).
    pub(crate) eight_mhz_fast: bool,
    /// Per-access GVRAM wait accumulator (units of 0x100 per access).
    pub(crate) gvram_access_count: i64,
    /// GVRAM accumulator limit for reads.
    pub(crate) gvram_access_limit_read: i64,
    /// GVRAM accumulator limit for writes.
    pub(crate) gvram_access_limit_write: i64,
    /// Active display width in pixels.
    pub(crate) display_width: u32,
    /// Active display height in pixels.
    pub(crate) display_height: u32,
    /// Number assigned to the next published frame.
    pub(crate) presented_frames: u64,
    /// Level-1 kanji ROM. Supplies the built-in 8x8 ANK font at offset 0x1000.
    kanji1: Vec<u8>,
    /// MA level-2 kanji ROM, loaded and hash-validated. Read through the
    /// 0xEC/0xED I/O window.
    kanji2: Vec<u8>,
    /// Disk sub-CPU (PC80S31K) 64 KiB memory: disk.rom + RAM.
    pub(crate) sub_mem: SubMemory,
    /// Sub-CPU cycle position in sub-clock (4 MHz) T-states.
    pub(crate) sub_cycle: u64,
    /// Right-shift converting sub T-states to main-clock units (0 at 4 MHz main,
    /// 1 at 8 MHz main where one sub T-state is two main units).
    pub(crate) sub_to_main_shift: u32,
    /// Carry of main-clock units not yet spent as whole sub T-states, keeping the
    /// clock-ratio conversion exact across slices.
    pub(crate) sub_clock_credit: u64,
    /// Disk sub-CPU uPD765A FDC (driven by programmed I/O, no DMA).
    pub(crate) fdc: Upd765aFdc<UPD765_PLATFORM_STANDARD>,
    /// Floppy drive store (reused for mounting and sector access).
    pub(crate) floppy: FloppyController,
    /// PPI mailbox, host side (main I/O 0xFC-0xFF).
    pub(crate) ppi_main: I8255,
    /// PPI mailbox, disk side (sub I/O 0xFC-0xFF).
    pub(crate) ppi_sub: I8255,
    /// Port 0xF4 drive-mode latch (per-drive 2D/2DD/2HD selection).
    pub(crate) drive_mode: u8,
    /// Port 0xF8 motor state (bits 0/1 per drive).
    pub(crate) motor_on: u8,
    /// FDC terminal-count line asserted by a port 0xF8 read.
    pub(crate) tc_active: bool,
    /// Cycle until which the interleave runs tight (a PPI strobe needs the peer
    /// CPU to make prompt progress).
    pub(crate) resync_until: u64,
    /// Cycle interval (main-clock units) between PIO data-rate DRQ ticks.
    pub(crate) drq_byte_cycles: u64,
    clocks: ClockConfig,
    model: Pc8801Model,
    tracer: T,
}

impl<T: TraceSink> Pc8801Bus<T> {
    /// Returns the main CPU clock frequency in Hz.
    pub fn cpu_clock_hz(&self) -> u32 {
        self.clocks.main_clock_hz
    }

    /// Returns the disk sub-CPU clock frequency in Hz.
    pub fn sub_clock_hz(&self) -> u32 {
        self.clocks.sub_clock_hz
    }

    /// Returns a reference to the tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Returns a mutable reference to the tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Reads a byte through the current main-CPU memory decode.
    ///
    /// Intended for tests and debugging; the bank-control register state
    /// determines which source is observed.
    pub fn peek_byte(&mut self, address: u16) -> u8 {
        self.memory.read_byte(address)
    }

    /// Returns the raw main RAM image.
    ///
    /// Intended for tests and debugging where observing RAM through the current
    /// bank-control state would disturb or obscure the data being inspected.
    pub fn main_ram(&self) -> &[u8] {
        &self.memory.state.ram[..]
    }

    /// Returns the port 0x53 layer-disable latch.
    pub fn layer_disable(&self) -> u8 {
        self.layer_disable
    }

    /// Returns text DMA and CRTC state for tests and debugging.
    pub fn text_display_debug_state(&self) -> (u16, i32, u8, bool, u8, usize, u8, u8, u8) {
        let channel = self.dma.channels[TEXT_CHANNEL];
        (
            channel.address,
            channel.count,
            self.dma.mode,
            channel.running,
            self.crtc.status,
            self.crtc.buffer_ptr,
            self.crtc.columns,
            self.crtc.rows,
            self.crtc.char_height,
        )
    }

    /// Writes a byte through the current main-CPU memory decode.
    ///
    /// Intended for tests and tooling; the bank-control register state determines
    /// the destination (a selected GVRAM plane, the ALU path, RAM, etc.).
    pub fn poke_byte(&mut self, address: u16, value: u8) {
        self.memory.write_byte(address, value);
    }

    /// Loads the N88-BASIC main ROM image (32 KiB), mapped at 0x0000-0x7FFF in
    /// N88 mode. Useful for tests that drive the main CPU from a crafted image.
    pub fn load_main_rom(&mut self, data: &[u8]) {
        self.memory.load_n88_rom(data);
    }

    /// Returns the configured machine model.
    pub fn model(&self) -> Pc8801Model {
        self.model
    }

    /// Writes to a main-CPU I/O port. Intended for tests and tooling.
    pub fn io_write(&mut self, port: u16, value: u8) -> bool {
        self.main_io_write(port, value)
    }

    /// Reads a main-CPU I/O port. Intended for tests and tooling.
    pub fn io_read(&mut self, port: u16) -> (u8, bool) {
        self.main_io_read(port)
    }

    /// Injects a received byte into the USART and raises the RXRDY interrupt.
    /// Intended for tests (the RS-232C port is otherwise modeled as no-cable).
    pub fn inject_serial_byte(&mut self, byte: u8) {
        self.serial.push_received_byte(byte);
        self.pic.set_request(LEVEL_RXRDY);
    }

    /// Overrides the host BCD local-time source used by the RTC. Intended for
    /// tests that need a deterministic clock.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
    }

    /// Sets the N88-BASIC boot mode (DIP setting), supplied by the application
    /// configuration after construction.
    pub fn set_boot_mode(&mut self, boot_mode: BootMode) {
        self.memory.set_boot_mode(boot_mode);
    }

    /// Selects the display monitor timing, supplied by the application
    /// configuration after construction. In `Auto` mode the timing follows the
    /// software-selected line mode; the fixed modes force 15 or 24 kHz.
    pub fn set_monitor_timing(&mut self, timing: MonitorTiming) {
        self.monitor_timing = timing;
        self.apply_monitor_timing();
    }

    /// Selects the memory-wait compatibility switch, supplied by the application
    /// configuration after construction.
    pub fn set_memory_wait(&mut self, switch: MemoryWaitSwitch) {
        self.mem_wait_on = matches!(switch, MemoryWaitSwitch::Compatible);
    }

    /// Selects the 8 MHz wait mode, supplied by the application configuration
    /// after construction.
    pub fn set_eight_mhz_wait(&mut self, mode: EightMhzWaitMode) {
        self.eight_mhz_fast = matches!(mode, EightMhzWaitMode::Fast);
    }

    /// Resolves the effective `hireso` flag from the monitor-timing selection and
    /// the current line mode, recomputing the per-scanline period when it changes.
    fn apply_monitor_timing(&mut self) {
        let hireso = match self.monitor_timing {
            MonitorTiming::Auto => self.line_400(),
            MonitorTiming::Fixed15kHz => false,
            MonitorTiming::Fixed24kHz => true,
        };
        if self.hireso != hireso {
            self.hireso = hireso;
            self.recompute_crtc_timing();
        }
    }

    /// Returns whether the display currently uses the 24 kHz (hireso) monitor
    /// timing.
    pub fn monitor_is_hireso(&self) -> bool {
        self.hireso
    }

    /// Returns the current per-scanline period in main-CPU cycles.
    pub fn crtc_line_period(&self) -> u64 {
        self.crtc_line_period
    }

    /// Sets a key in the 16x8 keyboard matrix. The matrix is active low, so a
    /// pressed key clears its column bit in the row read at ports 0x00-0x0F.
    pub fn set_key(&mut self, row: usize, column: usize, pressed: bool) {
        if row >= self.keyboard_rows.len() || column >= 8 {
            return;
        }
        let mask = 1u8 << column;
        if pressed {
            self.keyboard_rows[row] &= !mask;
        } else {
            self.keyboard_rows[row] |= mask;
        }
    }

    /// Accumulates relative mouse movement. The latched delta is sampled the next
    /// time the readout sequence restarts (see the port 0x40 strobe).
    pub fn set_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.mouse_x += i32::from(delta_x);
        self.mouse_y += i32::from(delta_y);
    }

    /// Sets the mouse button state on the SSG read port (active low).
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        let mut buttons = 0xFFu8;
        if left {
            buttons &= !MOUSE_BUTTON_LEFT;
        }
        if right {
            buttons &= !MOUSE_BUTTON_RIGHT;
        }
        self.mouse_buttons = buttons;
        self.update_joyport();
    }

    /// Sets the digital joystick state. The joystick shares the OPN port A/B
    /// read lines with the mouse (they are the same physical connector), so the
    /// directions only reach port A while the mouse is not actively being read.
    pub fn set_joystick(&mut self, state: JoystickState) {
        let mut port_a = 0xFFu8;
        if state.up {
            port_a &= !JOYSTICK_UP;
        }
        if state.down {
            port_a &= !JOYSTICK_DOWN;
        }
        if state.left {
            port_a &= !JOYSTICK_LEFT;
        }
        if state.right {
            port_a &= !JOYSTICK_RIGHT;
        }
        let mut port_b = 0xFFu8;
        if state.trigger1 {
            port_b &= !JOYSTICK_TRIGGER1;
        }
        if state.trigger2 {
            port_b &= !JOYSTICK_TRIGGER2;
        }
        self.joystick_port_a = port_a;
        self.joystick_port_b = port_b;
        self.update_joyport();
    }

    /// Writes bytes into text VRAM starting at `offset`. Intended for tests.
    pub fn write_tvram(&mut self, offset: usize, data: &[u8]) {
        for (index, byte) in data.iter().enumerate() {
            let target = offset + index;
            if target < self.memory.state.tvram.len() {
                self.memory.state.tvram[target] = *byte;
            }
        }
    }

    /// Returns the expanded per-cell character codes from the CRTC. Intended for
    /// tests.
    pub fn crtc_text_expand(&self) -> &[u8] {
        self.crtc.text_expand()
    }

    /// Returns the expanded per-cell attributes from the CRTC. Intended for tests.
    pub fn crtc_attrib_expand(&self) -> &[u8] {
        self.crtc.attrib_expand()
    }

    /// Returns the current main-CPU cycle count.
    pub fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Returns a read-only snapshot reference to the disk FDC state.
    ///
    /// Intended for integration tests and debugging long-running disk workflows.
    pub fn fdc_state(&self) -> &device::upd765a_fdc::Upd765aFdcState {
        &self.fdc.state
    }

    /// Returns the cycle of the next scheduled event, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        if self.next_event_cycle == u64::MAX {
            None
        } else {
            Some(self.next_event_cycle)
        }
    }

    /// Advances the cycle clock, processing any events that have come due.
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;

        if cycle >= self.next_event_cycle {
            self.process_events();
        }
    }

    /// Returns the packed RGBA text framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// Returns the active display dimensions in pixels.
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// Returns the character-generator ROM (the level-1 kanji ROM).
    pub fn font_rom_data(&self) -> &[u8] {
        &self.kanji1
    }

    fn update_next_event_cycle(&mut self) {
        self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
    }

    /// Enables or disables the CD-ROM BIOS ROM bank at 0x0000-0x7FFF. The MC
    /// resets with it enabled (the CD-System BIOS runs first); the CD BIOS turns
    /// it off via port 0x99 once it hands control to N88/disk boot.
    pub fn set_cdrom_bios_bank(&mut self, enabled: bool) {
        self.memory.state.cdrom_bank = enabled;
    }

    /// Inserts a CD-ROM disc image into the CD-ROM drive.
    pub fn insert_cdrom(&mut self, image: CdImage) {
        self.cdrom.insert(image);
    }

    /// Ejects the CD-ROM disc, if any.
    pub fn eject_cdrom(&mut self) {
        self.cdrom.eject();
    }

    /// Generates and mixes one audio frame from the OPNA and the beeper into
    /// `output` (interleaved stereo), returning the number of samples written.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let current_cycle = self.current_cycle;
        let cpu_clock_hz = self.clocks.main_clock_hz;
        let sample_rate = self.clocks.sample_rate;
        // The beeper writes the buffer (it has no system PIT, so the main clock
        // drives its period); the OPNA then mixes on top, matching the PC-98
        // ordering where the beeper runs first.
        let count = self.beeper.generate_samples(
            current_cycle,
            cpu_clock_hz,
            cpu_clock_hz,
            sample_rate,
            volume,
            output,
        );
        self.soundboard_ii
            .generate_samples(current_cycle, cpu_clock_hz, volume, output);
        self.cdrom.generate_audio_samples(volume, output);
        self.apply_sound_timers();
        count
    }

    /// Drains the OPNA's pending FM timer requests onto the scheduler and routes
    /// its IRQ edge to INT4 (gated by SINTM).
    fn apply_sound_timers(&mut self) {
        // At most two timer actions; copy out to release the device borrow.
        let timers: [Option<FmTimerAction>; 2] = {
            let actions = self.soundboard_ii.drain_timers();
            let mut out = [None, None];
            for (slot, action) in out.iter_mut().zip(actions.iter()) {
                *slot = Some(*action);
            }
            out
        };
        for action in timers.into_iter().flatten() {
            let (timer_id, fire_cycle) = match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => (timer_id, Some(fire_cycle)),
                FmTimerAction::Cancel { timer_id } => (timer_id, None),
            };
            let kind = if timer_id == 0 {
                Event88::FmTimerA
            } else {
                Event88::FmTimerB
            };
            match fire_cycle {
                Some(cycle) => self.scheduler.schedule(kind, cycle),
                None => self.scheduler.cancel(kind),
            }
        }
        if self.soundboard_ii.take_irq_change().is_some() {
            self.recompute_sound_irq();
        }
        self.update_next_event_cycle();
    }

    /// Routes the OPNA IRQ output to i8214 level 4 (INT4), masked when SINTM
    /// (port 0x32 bit 7) is set.
    fn recompute_sound_irq(&mut self) {
        let sintm_clear = self.memory.state.misc_ctrl & MISC_CTRL_SINTM == 0;
        if self.soundboard_ii.irq_asserted() && sintm_clear {
            self.pic.set_request(LEVEL_INT4);
        } else {
            self.pic.clear_request(LEVEL_INT4);
        }
    }

    fn row_period(&self) -> u64 {
        self.crtc_line_period * u64::from(self.crtc.char_height)
    }

    fn process_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        for index in 0..due.len() {
            let event = due[index];
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
                Event88::ClockTimer => {
                    self.pic.set_request(LEVEL_CLOCK);
                    self.scheduler.schedule(
                        Event88::ClockTimer,
                        event.fire_cycle + self.clock_timer_period,
                    );
                }
                Event88::CrtcDisplayStart => self.on_display_start(event.fire_cycle),
                Event88::CrtcVline => self.on_vline(event.fire_cycle),
                Event88::CrtcVsync => self.on_vsync(event.fire_cycle),
                Event88::FdcDrqByte => self.on_fdc_drq_byte(event.fire_cycle),
                Event88::FdcSeekComplete => self.fdc.state.interrupt_pending = true,
                Event88::FdcTcClear => {
                    self.tc_active = false;
                    self.fdc.state.tc = false;
                }
                Event88::FmTimerA => {
                    self.soundboard_ii.timer_expired(0, event.fire_cycle);
                    self.apply_sound_timers();
                }
                Event88::FmTimerB => {
                    self.soundboard_ii.timer_expired(1, event.fire_cycle);
                    self.apply_sound_timers();
                }
                Event88::CrtcBusRequestEnd
                | Event88::FdcPhaseComplete
                | Event88::FdcDataLost
                | Event88::FdcResult
                | Event88::FdcIndexPulse
                | Event88::BeepToggle => {}
            }
        }
        self.update_next_event_cycle();
    }

    /// Start of active display: reset the CRTC capture buffer, start the text
    /// DMA, fetch the first character row, and schedule the remaining rows plus
    /// the end-of-display vsync.
    fn on_display_start(&mut self, fire_cycle: u64) {
        self.crtc.start_frame();
        self.vrtc_active = false;
        self.crtc_current_row = 0;

        let display_enabled = self.crtc.status & STATUS_DISPLAY_ENABLE != 0;
        if display_enabled {
            self.dma.start(TEXT_CHANNEL);
            if self.dma.channels[TEXT_CHANNEL].running {
                self.crtc.state.status &= !STATUS_UNDERRUN;
            } else {
                self.crtc.state.status |= STATUS_UNDERRUN;
            }
            self.recompute_busreq_clocks();
            // Row 0 is fetched at the start of display (v == 0).
            let bytes_per_row = self.crtc.bytes_per_row();
            self.run_text_dma_row(bytes_per_row);
            self.assert_busreq(fire_cycle);
        }
        self.crtc_current_row = 1;

        let row_period = self.row_period();
        if u32::from(self.crtc.rows) > 1 {
            self.scheduler
                .schedule(Event88::CrtcVline, fire_cycle + row_period);
        }
        let display_period = u64::from(self.crtc.display_lines()) * self.crtc_line_period;
        self.scheduler
            .schedule(Event88::CrtcVsync, fire_cycle + display_period);
    }

    /// Per-character-row tick: feed one row of characters and attribute strips to
    /// the CRTC through the text DMA.
    fn on_vline(&mut self, fire_cycle: u64) {
        let rows = u32::from(self.crtc.rows);
        if self.crtc_current_row < rows && self.dma.channels[TEXT_CHANNEL].running {
            let bytes_per_row = self.crtc.bytes_per_row();
            self.run_text_dma_row(bytes_per_row);
            self.assert_busreq(fire_cycle);
        }
        self.crtc_current_row += 1;
        if self.crtc_current_row < rows {
            let row_period = self.row_period();
            self.scheduler
                .schedule(Event88::CrtcVline, fire_cycle + row_period);
        }
    }

    /// End of active display: finish the DMA, expand the captured buffer into the
    /// per-cell planes, raise the VRTC interrupt, render the frame, and schedule
    /// the next frame.
    fn on_vsync(&mut self, fire_cycle: u64) {
        if self.dma.channels[TEXT_CHANNEL].running {
            self.finish_text_dma();
        }
        let line_400 = self.line_400();
        self.crtc.expand_buffer(self.hireso, line_400);
        self.crtc.finish_frame();

        // VRTC is raised by machine wiring; the i8214 gates it via port 0xE6.
        self.pic.set_request(LEVEL_VRTC);
        self.vrtc_active = true;
        self.crtc.update_blink();

        self.render_frame();
        self.trace_presentation();

        let vblank_lines = u64::from(self.crtc.vretrace) * u64::from(self.crtc.char_height);
        let vblank_period = vblank_lines * self.crtc_line_period;
        self.scheduler
            .schedule(Event88::CrtcDisplayStart, fire_cycle + vblank_period);
    }

    /// Drains CD-ROM read data into memory over DMA channel 1 while the SCSI
    /// target is presenting bytes, DMA is enabled, and the channel has counts
    /// left. No-op unless all three hold, so it is safe to call after any
    /// CD-ROM port access.
    fn run_cdrom_dma(&mut self) {
        if !self.cdrom.dma_request() {
            return;
        }
        // The SCSI DREQ drives DMA channel 1: begin the channel (honouring its
        // mode-register enable) if it is not already running.
        if !self.dma.channels[CDROM_DMA_CHANNEL].running {
            self.dma.start(CDROM_DMA_CHANNEL);
        }
        while self.cdrom.dma_request() && self.dma.channel_active(CDROM_DMA_CHANNEL) {
            let address = self.dma.channel_address(CDROM_DMA_CHANNEL);
            let byte = self.cdrom.dma_read_byte();
            self.memory.state.ram[address as usize] = byte;
            self.dma.channel_advance(CDROM_DMA_CHANNEL);
        }
        if self.dma.channels[CDROM_DMA_CHANNEL].running
            && !self.dma.channel_active(CDROM_DMA_CHANNEL)
        {
            self.dma.finish(CDROM_DMA_CHANNEL);
        }
    }

    fn run_text_dma_row(&mut self, mut byte_count: u32) {
        while byte_count > 0 && self.dma.channel_active(TEXT_CHANNEL) {
            let address = self.dma.channel_address(TEXT_CHANNEL);
            let byte = self.read_dma_byte(address);
            self.crtc.push_dma_byte(byte);
            self.dma.channel_advance(TEXT_CHANNEL);
            byte_count -= 1;
        }
    }

    fn finish_text_dma(&mut self) {
        while self.dma.channel_active(TEXT_CHANNEL) {
            let address = self.dma.channel_address(TEXT_CHANNEL);
            let byte = self.read_dma_byte(address);
            self.crtc.push_dma_byte(byte);
            self.dma.channel_advance(TEXT_CHANNEL);
        }
        self.dma.finish(TEXT_CHANNEL);
    }

    /// Reads a byte for the text DMA from the raw backing store, bypassing the
    /// CPU bank decode. In V1H/V2 the 0xF000-0xFFFF window reads text VRAM.
    fn read_dma_byte(&self, address: u16) -> u8 {
        let uses_tvram = matches!(self.memory.state.boot_mode, BootMode::V1H | BootMode::V2);
        if uses_tvram && (address & 0xF000) == 0xF000 {
            self.memory.state.tvram[(address & 0x0FFF) as usize]
        } else {
            self.memory.state.ram[address as usize]
        }
    }

    fn line_400(&self) -> bool {
        self.memory.state.gfx_ctrl & GFX_CTRL_400LINE_MASK == 0
    }

    fn recompute_crtc_timing(&mut self) {
        let horizontal_freq = if self.hireso {
            HORIZONTAL_FREQ_24KHZ
        } else {
            HORIZONTAL_FREQ_15KHZ
        };
        self.crtc_line_period = (u64::from(self.clocks.main_clock_hz) / horizontal_freq).max(1);
    }

    /// Recomputes the per-character-row text-DMA bus-hold window in CPU cycles.
    ///
    /// The per-scanline hold is
    /// `(total_dma_bytes) * cycles_per_byte / display_scanlines`; aggregated to one
    /// character row that reduces to `bytes_per_row * cycles_per_byte`. The
    /// per-byte cost is 5.95 cycles at 4 MHz and 10.58 cycles at 8 MHz.
    fn recompute_busreq_clocks(&mut self) {
        let bytes_per_row = u64::from(self.crtc.bytes_per_row());
        let cycles_per_byte_hundredths = if self.cpu_clock_low { 595 } else { 1058 };
        self.busreq_clocks = (bytes_per_row * cycles_per_byte_hundredths + 50) / 100;
    }

    /// Asserts the text-DMA BUSREQ lockout for the current character row. Only
    /// V1S/N boot modes halt the CPU during display; V1H/V2 do not.
    fn assert_busreq(&mut self, fire_cycle: u64) {
        if matches!(self.memory.state.boot_mode, BootMode::V1S)
            || self.memory.state.boot_mode.is_n_family()
        {
            self.busreq_until = fire_cycle + self.busreq_clocks;
            self.scheduler
                .schedule(Event88::CrtcBusRequestEnd, self.busreq_until);
        }
    }

    /// Returns whether the main CPU is currently locked off the bus by the text DMA.
    pub fn busreq_active(&self) -> bool {
        self.current_cycle < self.busreq_until
    }

    /// Returns the cycle until which the main CPU is locked off the bus.
    pub fn busreq_until(&self) -> u64 {
        self.busreq_until
    }

    /// Memory access-wait cycles for a CPU access, selected by the decoded
    /// access target and the current memory-wait configuration.
    fn access_wait(&mut self, target: Pc8801MemoryTarget, read: bool) -> i64 {
        match target {
            Pc8801MemoryTarget::MainRam
            | Pc8801MemoryTarget::BasicRom
            | Pc8801MemoryTarget::ExtensionRam
            | Pc8801MemoryTarget::TextWindow => self.main_wait(read),
            Pc8801MemoryTarget::TextVram => self.tvram_wait(read),
            Pc8801MemoryTarget::DictionaryRom => self.dictionary_wait(),
            Pc8801MemoryTarget::GvramPlane | Pc8801MemoryTarget::GvramAlu => {
                self.gvram_wait(read) + self.insert_gvram_wait(read)
            }
        }
    }

    /// Wait cycles for main RAM, ROM, extension RAM, and text-window accesses.
    fn main_wait(&self, read: bool) -> i64 {
        if self.cpu_clock_low {
            i64::from(self.mem_wait_on && read)
        } else {
            i64::from(!self.eight_mhz_fast) + i64::from(self.mem_wait_on)
        }
    }

    /// Wait cycles for text VRAM accesses at 0xF000-0xFFFF.
    fn tvram_wait(&self, read: bool) -> i64 {
        if self.cpu_clock_low {
            i64::from(self.mem_wait_on && read)
        } else if read {
            2
        } else {
            1
        }
    }

    /// Wait cycles for dictionary ROM reads.
    fn dictionary_wait(&self) -> i64 {
        if self.cpu_clock_low {
            i64::from(self.mem_wait_on)
        } else {
            2
        }
    }

    /// Per-access GVRAM accumulator. Only graphics-on accesses accumulate; when
    /// the accumulator crosses the clock-dependent limit it yields one extra
    /// wait cycle.
    fn insert_gvram_wait(&mut self, read: bool) -> i64 {
        if self.memory.state.gfx_ctrl & GFX_CTRL_GRPHE == 0 {
            return 0;
        }
        let limit = if read {
            self.gvram_access_limit_read
        } else {
            self.gvram_access_limit_write
        };
        self.gvram_access_count += GVRAM_ACCESS_INCREMENT;
        if self.gvram_access_count >= limit {
            self.gvram_access_count -= limit;
            1
        } else {
            0
        }
    }

    /// GVRAM/ALU display-time access wait. The large V1S display-time penalty
    /// models the CPU sharing the GVRAM bus with the display refresh; GHSM
    /// (high-speed mode) and vertical retrace remove it.
    fn gvram_wait(&self, read: bool) -> i64 {
        let graphics_on = self.memory.state.gfx_ctrl & GFX_CTRL_GRPHE != 0;
        let high_speed = self.port40 & PORT40_GHSM != 0;
        let vblank = self.vrtc_active;
        let standard_mode = matches!(
            self.memory.state.boot_mode,
            BootMode::V1S | BootMode::N | BootMode::N80
        );

        if !graphics_on {
            return if self.cpu_clock_low {
                i64::from(read && self.mem_wait_on)
            } else {
                3
            };
        }

        if self.cpu_clock_low {
            if standard_mode && !high_speed && !vblank {
                if self.hireso { 68 } else { 114 }
            } else if vblank {
                0
            } else {
                2
            }
        } else if standard_mode && !high_speed && !vblank {
            if self.hireso { 90 } else { 141 }
        } else if vblank {
            3
        } else {
            5
        }
    }

    /// Extra Z80 M1 opcode-fetch wait cycles, applied only on instruction
    /// fetches (not operand fetches). Only relevant when the memory-wait switch
    /// is off.
    fn m1_wait(&self, address: u16) -> i64 {
        if self.mem_wait_on {
            return 0;
        }
        match self.memory.state.boot_mode {
            BootMode::V1S | BootMode::N | BootMode::N80 => i64::from(self.cpu_clock_low),
            BootMode::V1H | BootMode::V2 | BootMode::N80SR => i64::from(
                self.cpu_clock_low
                    && address >= HIGH_REGION_M1_START
                    && self.memory.high_region_opcode_fetch_uses_tvram_wait(),
            ),
        }
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

    fn render_frame(&mut self) {
        let color_mode = self.port30 & PORT30_COLOR == 0;
        let width_40col = self.port30 & PORT30_WIDTH40 == 0;
        let text_enabled = self.crtc.status & STATUS_DISPLAY_ENABLE != 0
            && self.layer_disable & LAYER_DISABLE_TEXT == 0;
        let char_height = u32::from(self.crtc.char_height);
        let columns = u32::from(self.crtc.columns);
        let rows = u32::from(self.crtc.rows);
        let background_rgb = self.palette.pens[BACKGROUND_PEN].to_rgb();

        let gfx_ctrl = self.memory.state.gfx_ctrl;
        let graphics_enabled = gfx_ctrl & GFX_CTRL_GRPHE != 0;
        let line_400 = self.line_400();
        let graphics_mode = if gfx_ctrl & GFX_CTRL_HCOLOR != 0 {
            GraphicsMode88::Color8
        } else if line_400 {
            GraphicsMode88::Attrib400
        } else {
            GraphicsMode88::Attrib200
        };
        let palette_mode = self.memory.state.misc_ctrl & MISC_CTRL_PMODE != 0;
        let plane_disable = (self.layer_disable >> 1) & 0x07;

        let mut graphics_palette = [[0u8; 3]; 8];
        for (pen, rgb) in graphics_palette.iter_mut().enumerate() {
            *rgb = self.palette.pens[pen].to_rgb();
        }

        let width = PC88_WIDTH as u32;
        // The output height follows the CRTC scanline count, except a 400-line
        // graphics mode forces the full 400-line surface (with the 200-line text
        // grid doubled into it by the compositor).
        let height = if graphics_enabled && graphics_mode == GraphicsMode88::Attrib400 {
            PC88_MAX_HEIGHT as u32
        } else {
            (char_height * rows).min(PC88_MAX_HEIGHT as u32)
        };
        self.display_width = width;
        self.display_height = height;

        let Pc8801Bus {
            crtc,
            renderer,
            memory,
            ..
        } = self;
        let gvram = &memory.state.gvram[..];
        let inputs = RenderInputs88 {
            text_codes: crtc.text_expand(),
            text_attrib: crtc.attrib_expand(),
            columns,
            rows,
            char_height,
            width_40col,
            color_mode,
            text_enabled,
            background_rgb,
            graphics_enabled,
            graphics_mode,
            line_400,
            gvram_blue: &gvram[0..GVRAM_PLANE_SIZE],
            gvram_red: &gvram[GVRAM_PLANE_SIZE..GVRAM_PLANE_SIZE * 2],
            gvram_green: &gvram[GVRAM_PLANE_SIZE * 2..GVRAM_PLANE_SIZE * 3],
            graphics_palette,
            palette_mode,
            plane_disable,
            width,
            height,
        };
        renderer.render(&inputs);
    }

    fn main_io_read(&mut self, port: u16) -> (u8, bool) {
        let value = match port & 0xFF {
            0x00..=0x0F => self.keyboard_read(port),
            0x20 => self.serial_read_data(),
            0x21 => self.serial.read_status(),
            0x30 => self.port30_in(),
            0x31 => self.port31_in(),
            0x32 if self.memory.state.boot_mode.is_n80_family() => OPEN_BUS,
            0x32 => self.memory.state.misc_ctrl,
            0x33 if self.memory.state.boot_mode.is_n80_family() => self.port33_in(),
            0x40 => self.port40_read(),
            0x44 => self.soundboard_ii.read_status(self.current_cycle),
            0x45 => self.soundboard_ii.read_data(self.current_cycle),
            0x46 => self.soundboard_ii.read_status_hi(self.current_cycle),
            0x47 => self.soundboard_ii.read_data_hi(self.current_cycle),
            0x50 => self.crtc.read_parameter(),
            0x51 => self.crtc.read_status(),
            0x5C => self.gvram_select_read(),
            0x60..=0x68 => self.dma.read_io(port & 0x0F),
            0x90 => self.cdrom.read_status(),
            0x91 => self.cdrom.read_data(),
            0x98 => self.cdrom.read_clock(),
            0x99 => self.cdrom.read_id(),
            0x9B => self.cdrom.read_volume_meter(1),
            0x9D => self.cdrom.read_volume_meter(0),
            0x6E => self.cpu_clock_read(),
            0x6F => self.baud_rate | 0xF0,
            0x70 => self.memory.state.window_bank,
            0x71 => self.memory.state.ext_rom_bank,
            0xE2 => !self.memory.state.extram_mode | 0xEE,
            0xE3 => self.memory.state.extram_bank | 0xF0,
            0xE8 => self.kanji_read(&self.kanji1, self.kanji1_addr, true),
            0xE9 => self.kanji_read(&self.kanji1, self.kanji1_addr, false),
            0xEC => self.kanji_read(&self.kanji2, self.kanji2_addr, true),
            0xED => self.kanji_read(&self.kanji2, self.kanji2_addr, false),
            0xFC..=0xFF => self.ppi_main.read((port & 0x03) as u8),
            _ => return (OPEN_BUS, false),
        };
        (value, true)
    }

    /// Reads a kanji ROM byte through the I/O window. Each latched code selects a
    /// 2-byte word; `high` returns the odd byte (`code*2+1`), otherwise the even
    /// byte (`code*2`).
    fn kanji_read(&self, rom: &[u8], code: u16, high: bool) -> u8 {
        if rom.is_empty() {
            return OPEN_BUS;
        }
        let offset = code as usize * 2 + usize::from(high);
        rom[offset % rom.len()]
    }

    /// Recomposes the uPD4990A command byte from the latched port 0x10 lines
    /// (C0/C1/C2 and DIN) and the port 0x40 lines (STB/CLK) and drives the RTC.
    /// The chip does its own edge detection, so this is safe to call on every
    /// port write.
    fn rtc_strobe(&mut self) {
        let mut command = self.port10 & PORT10_RTC_COMMAND;
        if self.port40 & PORT40_RTC_STB != 0 {
            command |= RTC_CHIP_STB;
        }
        if self.port40 & PORT40_RTC_CLK != 0 {
            command |= RTC_CHIP_CLK;
        }
        if self.port10 & PORT10_RTC_DIN != 0 {
            command |= RTC_CHIP_DIN;
        }
        let host_time = (self.host_date_time_provider)().to_bcd_bytes();
        self.rtc.write_port(command, &host_time);
    }

    /// Reads the USART data register, updating the RXRDY interrupt request as the
    /// receive FIFO drains.
    fn serial_read_data(&mut self) -> u8 {
        let (data, clear_irq, retrigger_irq) = self.serial.read_data();
        if clear_irq {
            self.pic.clear_request(LEVEL_RXRDY);
        }
        if retrigger_irq {
            self.pic.set_request(LEVEL_RXRDY);
        }
        data
    }

    fn keyboard_read(&self, port: u16) -> u8 {
        let row = (port & 0x0F) as usize;
        let value = self.keyboard_rows[row];
        if row == 0x0E { value & 0x7F } else { value }
    }

    /// Handles a mouse latch strobe edge (port 0x40 bit 6). Each edge advances
    /// the readout phase; a gap longer than the timeout restarts at the X high
    /// nibble. On the restart edge the accumulated movement is latched as a pair
    /// of signed byte deltas (old minus new, the standard mouse sign convention).
    fn mouse_strobe(&mut self, port40: u8) {
        let level = port40 & PORT40_MOUSE_STROBE != 0;
        if level == self.mouse_strobe_level {
            return;
        }
        self.mouse_strobe_level = level;
        self.mouse_strobe_seen = true;

        let now = self.current_cycle;
        if now.wrapping_sub(self.mouse_strobe_cycle) > self.mouse_timeout_cycles {
            self.mouse_phase = 3;
        }
        self.mouse_strobe_cycle = now;
        self.mouse_phase = (self.mouse_phase + 1) & 0x03;

        if self.mouse_phase == 0 {
            let delta_x = (self.mouse_latch_x - self.mouse_x) as i8 as u8;
            let delta_y = (self.mouse_latch_y - self.mouse_y) as i8 as u8;
            self.mouse_data = (u16::from(delta_x) << 8) | u16::from(delta_y);
            self.mouse_latch_x = self.mouse_x;
            self.mouse_latch_y = self.mouse_y;
        }
        self.update_joyport();
    }

    /// Whether the mouse currently owns the shared OPN port A line. The mouse
    /// owns it only after a strobe edge and until the readout sequence times
    /// out; otherwise the joystick directions drive port A.
    fn mouse_owns_port(&self) -> bool {
        self.mouse_strobe_seen
            && self.current_cycle.wrapping_sub(self.mouse_strobe_cycle) <= self.mouse_timeout_cycles
    }

    /// Recomputes the OPN port A/B read values and presents them on the chip
    /// (registers 0x0E/0x0F). Port A carries the mouse readout nibble while the
    /// mouse owns the port, otherwise the joystick directions. Port B is the
    /// combined (active-low) mouse buttons and joystick triggers.
    fn update_joyport(&mut self) {
        let mouse_buttons = ((self.mouse_buttons >> 4) & 0x03) | 0xFC;
        let port_b = mouse_buttons & self.joystick_port_b;
        let port_a = if self.mouse_owns_port() {
            let shift = 4 * (3 - u16::from(self.mouse_phase));
            let nibble = ((self.mouse_data >> shift) & 0x0F) as u8;
            nibble | 0xF0
        } else {
            self.joystick_port_a
        };
        self.soundboard_ii.set_joyport(port_a, port_b);
    }

    /// Port 0x30 read: DIP switches. Bits 0-1 distinguish the PC-8001-compatible
    /// N-family personalities: N=2, N80=3, N80SR=1.
    ///
    /// The text width/color come from the separate write latch.
    fn port30_in(&self) -> u8 {
        let mut value = 0xC0;
        value |= match self.memory.state.boot_mode {
            BootMode::N => 0x02,
            BootMode::N80 => 0x03,
            BootMode::N80SR => 0x01,
            BootMode::V1S | BootMode::V1H | BootMode::V2 => 0x03,
        };
        value |= 0x08; // LN25: 25 text rows
        value |= 0x10; // SPRM
        value |= 0x20; // PDEL
        value
    }

    /// Port 0x31 read: boot mode in bits 7:6 plus USART DIP defaults.
    fn port31_in(&self) -> u8 {
        let mut value = match self.memory.state.boot_mode {
            BootMode::V2 => 0x40,
            BootMode::V1H | BootMode::N80SR => 0xC0,
            BootMode::V1S | BootMode::N | BootMode::N80 => 0x80,
        };
        value |= 0x20; // HDPX: half duplex off
        value |= 0x10; // XPRM
        value |= 0x08; // ST2B: one stop bit
        value |= 0x01; // no parity
        value
    }

    fn port33_in(&self) -> u8 {
        self.memory.state.n80_ctrl & N80_CTRL_READ_MASK
    }

    /// Reads the port 0x40 "strobe port". Reports the VRTC state in bit 5, the RTC
    /// serial data out in bit 4, and the BOOT device select in bit 3; the remaining
    /// bits return fixed idle defaults.
    fn port40_read(&self) -> u8 {
        let mut value = PORT40_READ_BASE;
        if !self.floppy.has_drive(0) && !self.floppy.has_drive(1) {
            value |= PORT40_BOOT_ROM;
        }
        if self.vrtc_active {
            value |= PORT40_VRTC;
        }
        if self.rtc.cdat() != 0 {
            value |= PORT40_RTC_DOUT;
        }
        value
    }

    fn gvram_select_read(&self) -> u8 {
        let select = match self.memory.state.gvram_sel {
            GvramSelect::MainRam => 0x00,
            GvramSelect::Blue => 0x01,
            GvramSelect::Red => 0x02,
            GvramSelect::Green => 0x04,
        };
        select | 0xF8
    }

    fn cpu_clock_read(&self) -> u8 {
        let clock_bit = if self.cpu_clock_low { 0x80 } else { 0x00 };
        clock_bit | 0x10
    }

    fn main_io_write(&mut self, port: u16, value: u8) -> bool {
        match port & 0xFF {
            0x10 => {
                self.port10 = value;
                self.rtc_strobe();
            }
            0x20 => self.serial.write_data(value),
            0x21 => self.serial.write_command(value),
            0x30 => self.port30 = value,
            0x31 => {
                self.memory.state.gfx_ctrl = value;
                self.apply_monitor_timing();
            }
            0x32 if self.memory.state.boot_mode.is_n80_family() => {}
            0x32 => {
                self.memory.state.misc_ctrl = value;
                if value & MISC_CTRL_GVAM != 0 {
                    self.memory.state.gvram_sel = GvramSelect::MainRam;
                }
                self.recompute_sound_irq();
            }
            0x33 if self.memory.state.boot_mode.is_n80_family() => {
                self.memory.state.n80_ctrl = value & N80_CTRL_READ_MASK;
                if value & N80_CTRL_GVAM != 0 {
                    self.memory.state.misc_ctrl |= MISC_CTRL_GVAM;
                    self.memory.state.gvram_sel = GvramSelect::MainRam;
                } else {
                    self.memory.state.misc_ctrl &= !MISC_CTRL_GVAM;
                }
                if value & N80_CTRL_SINTM != 0 {
                    self.memory.state.misc_ctrl |= MISC_CTRL_SINTM;
                } else {
                    self.memory.state.misc_ctrl &= !MISC_CTRL_SINTM;
                }
                self.recompute_sound_irq();
            }
            0x34 => self.memory.state.alu_ctrl1 = value,
            0x35 => {
                self.memory.state.alu_ctrl2 = value;
                if self.memory.state.misc_ctrl & MISC_CTRL_GVAM != 0 {
                    self.memory.state.gvram_sel = GvramSelect::MainRam;
                }
            }
            0x40 => {
                self.port40 = value;
                self.beeper
                    .set_buzzer_enabled(value & PORT40_BEEP_ENABLE != 0, self.current_cycle);
                self.rtc_strobe();
                self.mouse_strobe(value);
            }
            0x44 => {
                self.soundboard_ii.write_address(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x45 => {
                self.soundboard_ii.write_data(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x46 => {
                self.soundboard_ii
                    .write_address_hi(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x47 => {
                self.soundboard_ii.write_data_hi(value, self.current_cycle);
                self.apply_sound_timers();
            }
            0x50 => {
                self.crtc.write_parameter(value);
                if self.crtc.timing_changed {
                    self.recompute_crtc_timing();
                    self.crtc.state.timing_changed = false;
                }
            }
            0x51 => self.crtc.write_command(value),
            0x52 => self.palette.write_background(value),
            0x53 if self.memory.state.boot_mode.is_n80_family() => {}
            0x53 => self.layer_disable = value,
            0x54..=0x5B => {
                let analog = self.memory.state.misc_ctrl & MISC_CTRL_PMODE != 0;
                self.palette
                    .write_pen((port & 0xFF) as usize - 0x54, value, analog);
            }
            0x5C => self.memory.state.gvram_sel = GvramSelect::Blue,
            0x5D => self.memory.state.gvram_sel = GvramSelect::Red,
            0x5E => self.memory.state.gvram_sel = GvramSelect::Green,
            0x5F => self.memory.state.gvram_sel = GvramSelect::MainRam,
            0x60..=0x68 => self.dma.write_io(port & 0x0F, value),
            0x90 => self.cdrom.write_select(value),
            0x91 => {
                self.cdrom.write_data(value);
                self.run_cdrom_dma();
            }
            0x94 => self.cdrom.write_reset(value),
            0x98 => self.cdrom.write_fader(value),
            0x99 => {
                let bank_enabled = self.cdrom.write_rom_bank(value);
                self.memory.state.cdrom_bank = bank_enabled;
            }
            0x9F => {
                self.cdrom.write_control(value);
                self.run_cdrom_dma();
            }
            0x6F => self.baud_rate = value & 0x0F,
            0x70 => self.memory.state.window_bank = value,
            0x71 => self.memory.state.ext_rom_bank = value,
            0x78 => {
                self.memory.state.window_bank = self.memory.state.window_bank.wrapping_add(1);
            }
            0xE2 => self.memory.state.extram_mode = value,
            0xE3 => self.memory.state.extram_bank = value,
            0xE4 => self.pic.write_priority(value),
            0xE6 => self.pic.write_mask(value),
            0xE8 => self.kanji1_addr = (self.kanji1_addr & 0xFF00) | u16::from(value),
            0xE9 => self.kanji1_addr = (self.kanji1_addr & 0x00FF) | (u16::from(value) << 8),
            0xEC => self.kanji2_addr = (self.kanji2_addr & 0xFF00) | u16::from(value),
            0xED => self.kanji2_addr = (self.kanji2_addr & 0x00FF) | (u16::from(value) << 8),
            0xF0 => self.memory.state.dic_bank = value,
            0xF1 => self.memory.state.dic_ctrl = value,
            0xFC => {
                self.ppi_main.write(0, value);
                self.ppi_sub.set_port_b(value);
            }
            0xFD => {
                self.ppi_main.write(1, value);
                self.ppi_sub.set_port_a(value);
            }
            0xFE | 0xFF => {
                let changed = self.ppi_main.write((port & 0x03) as u8, value);
                self.on_ppi_main_change(changed);
            }
            _ => return false,
        }
        true
    }
}

/// Main-CPU view over the bus: implements `common::Bus` with the main memory and
/// I/O maps. Built per run slice; never coexists with the sub-CPU view.
pub(crate) struct MainBusView<'a, T: TraceSink> {
    pub(crate) bus: &'a mut Pc8801Bus<T>,
}

impl<T: TraceSink> common::Bus for MainBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, target) = self.bus.memory.read_byte_with_access(address);
        self.bus.memory_wait_cycles += self.bus.access_wait(target, true);
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
                    true,
                ),
            );
        }
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address as u16;
        let target = self.bus.memory.write_byte_with_access(address, value);
        self.bus.memory_wait_cycles += self.bus.access_wait(target, false);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(u32::from(address)),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, target) = self.bus.memory.read_byte_with_access(address);
        self.bus.memory_wait_cycles += self.bus.access_wait(target, true);
        self.bus.memory_wait_cycles += self.bus.m1_wait(address);
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
                    true,
                ),
            );
        }
        value
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        let (value, handled) = self.bus.main_io_read(port);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_IO,
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
        let handled = self.bus.main_io_write(port, value);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_IO,
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
        self.bus.pic.has_pending_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        let acknowledge = self.bus.pic.acknowledge();
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::PC88_I8214,
                    common::TraceInterruptKind::Maskable,
                    acknowledge.level.map(u16::from),
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
        self.bus.set_current_cycle(cycle);
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.bus.memory_wait_cycles)
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.bus.tracer.yield_requested()
    }
}

impl<T: TraceSink> Pc8801Bus<T> {
    /// Whether the disk sub-CPU has a pending FDC interrupt.
    pub(crate) fn sub_irq_pending(&self) -> bool {
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
        }
    }

    /// Ejects the floppy from a drive and clears the FDC drive-occupied bit.
    pub(crate) fn eject_floppy(&mut self, drive: usize) {
        self.floppy.eject_drive(drive);
        if drive < 4 {
            self.fdc.state.drive_has_disk &= !(1 << drive);
        }
    }

    /// Flushes any dirty mounted floppies back to their source files.
    pub(crate) fn flush_floppies(&mut self) {
        self.floppy.flush_all_drives();
    }
}

/// Sub-CPU (PC80S31K) view over the bus: implements `common::Bus` with the disk
/// unit's memory and I/O maps. Built per run slice; never coexists with the
/// main-CPU view.
pub(crate) struct SubBusView<'a, T: TraceSink> {
    pub(crate) bus: &'a mut Pc8801Bus<T>,
}

impl<T: TraceSink> common::Bus for SubBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let value = self.bus.sub_mem.read(address);
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::sub_cpu(
                    self.bus.current_cycle,
                    self.bus.sub_cycle,
                    Some(u64::from(self.bus.sub_clock_hz())),
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
                    Some(u64::from(self.bus.sub_clock_hz())),
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
                    Some(u64::from(self.bus.sub_clock_hz())),
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
                    Some(u64::from(self.bus.sub_clock_hz())),
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
                    Some(u64::from(self.bus.sub_clock_hz())),
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
                    Some(u64::from(self.bus.sub_clock_hz())),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::PC88_SUB_FDC,
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

    // The sub CPU runs in its own clock domain (`sub_cycle`), not the shared
    // main-unit `current_cycle` that drives the scheduler.
    #[allow(clippy::misnamed_getters)]
    fn current_cycle(&self) -> u64 {
        self.bus.sub_cycle
    }

    fn set_current_cycle(&mut self, cycle: u64) {
        self.bus.set_sub_cycle(cycle);
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        0
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.bus.tracer.yield_requested()
    }
}

#[cfg(test)]
mod tests {
    use common::{TraceAccess, TraceInterrupt, trace_clock, trace_source};

    use super::*;
    use crate::config::ClockSelect;

    #[derive(Default)]
    struct ContextTrace {
        contexts: Vec<TraceContext>,
        accesses: Vec<TraceAccess>,
        interrupts: Vec<TraceInterrupt>,
    }

    impl TraceSink for ContextTrace {
        fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Access(access) => {
                    self.contexts.push(context);
                    self.accesses.push(access);
                }
                TraceEvent::Interrupt(interrupt) => self.interrupts.push(interrupt),
                _ => {}
            }
        }
    }

    #[test]
    fn main_and_sub_fetches_use_distinct_clock_domains() {
        let mut bus = Pc8801Bus::new_with_trace_sink(
            Pc8801Model::PC8801MC,
            ClockSelect::FourMhz,
            48_000,
            ContextTrace::default(),
        );
        {
            let mut main = MainBusView { bus: &mut bus };
            common::Bus::fetch_opcode_byte(&mut main, 0x1234);
        }
        {
            let mut sub = SubBusView { bus: &mut bus };
            common::Bus::fetch_opcode_byte(&mut sub, 0x0042);
        }

        let trace = bus.tracer();
        assert_eq!(trace.contexts[0].source, trace_source::CPU_MAIN);
        assert_eq!(trace.contexts[1].source, trace_source::CPU_SUB);
        assert_eq!(trace.contexts[1].clock_domain, trace_clock::CPU_SUB);
        assert_eq!(trace.accesses[0].kind, TraceAccessKind::Fetch);
        assert_eq!(trace.accesses[1].address, 0x0042);
    }

    #[test]
    fn traces_use_live_io_decode_and_interrupt_level() {
        let mut bus = Pc8801Bus::new_with_trace_sink(
            Pc8801Model::PC8801MC,
            ClockSelect::FourMhz,
            48_000,
            ContextTrace::default(),
        );
        {
            let mut main = MainBusView { bus: &mut bus };
            common::Bus::io_read_byte(&mut main, 0x30);
            common::Bus::io_read_byte(&mut main, 0xAA);
        }
        {
            let mut sub = SubBusView { bus: &mut bus };
            common::Bus::io_read_byte(&mut sub, 0xFA);
            common::Bus::io_read_byte(&mut sub, 0x01);
        }
        bus.pic.write_priority(8);
        bus.pic.set_request(LEVEL_INT4);
        {
            let mut main = MainBusView { bus: &mut bus };
            common::Bus::acknowledge_irq(&mut main);
        }

        assert!(bus.tracer().accesses[0].handled);
        assert!(!bus.tracer().accesses[1].handled);
        assert!(bus.tracer().accesses[2].handled);
        assert!(!bus.tracer().accesses[3].handled);
        assert_eq!(bus.tracer().interrupts[0].line, Some(u16::from(LEVEL_INT4)));
    }

    fn bus_4mhz() -> Pc8801Bus {
        Pc8801Bus::new(Pc8801Model::PC8801MC, ClockSelect::FourMhz, 48_000)
    }

    fn bus_8mhz() -> Pc8801Bus {
        Pc8801Bus::new(Pc8801Model::PC8801MC, ClockSelect::EightMhz, 48_000)
    }

    #[test]
    fn gvram_wait_matches_reference_table() {
        let mut bus = bus_4mhz();
        bus.memory.state.gfx_ctrl = GFX_CTRL_GRPHE;

        // V1S, 4 MHz, graphics on, not GHSM, active display: 114 cycles (15 kHz).
        bus.memory.state.boot_mode = BootMode::V1S;
        bus.vrtc_active = false;
        bus.port40 = 0;
        assert_eq!(bus.gvram_wait(true), 114);

        // GHSM removes the display-time penalty.
        bus.port40 = PORT40_GHSM;
        assert_eq!(bus.gvram_wait(true), 2);

        // During vertical retrace there is no penalty.
        bus.port40 = 0;
        bus.vrtc_active = true;
        assert_eq!(bus.gvram_wait(true), 0);

        // V2 and N80SR do not pay the V1/N80 display-time penalty.
        bus.set_boot_mode(BootMode::V2);
        bus.vrtc_active = false;
        assert_eq!(bus.gvram_wait(true), 2);
        bus.set_boot_mode(BootMode::N80SR);
        assert_eq!(bus.gvram_wait(true), 2);
    }

    #[test]
    fn gvram_wait_8mhz_v1s_active_display() {
        let mut bus = bus_8mhz();
        bus.memory.state.gfx_ctrl = GFX_CTRL_GRPHE;
        bus.memory.state.boot_mode = BootMode::V1S;
        bus.vrtc_active = false;
        bus.port40 = 0;
        assert_eq!(bus.gvram_wait(true), 141);
    }

    #[test]
    fn main_wait_follows_switches() {
        let mut bus = bus_4mhz();
        bus.mem_wait_on = true;
        assert_eq!(bus.main_wait(true), 1);
        assert_eq!(bus.main_wait(false), 0);
        bus.mem_wait_on = false;
        assert_eq!(bus.main_wait(true), 0);

        let mut bus = bus_8mhz();
        bus.eight_mhz_fast = true;
        bus.mem_wait_on = false;
        assert_eq!(bus.main_wait(true), 0);
        assert_eq!(bus.main_wait(false), 0);
        bus.eight_mhz_fast = false;
        bus.mem_wait_on = true;
        assert_eq!(bus.main_wait(true), 2);
        assert_eq!(bus.main_wait(false), 2);
    }

    #[test]
    fn tvram_wait_follows_switches() {
        let mut bus = bus_4mhz();
        bus.mem_wait_on = true;
        assert_eq!(bus.tvram_wait(true), 1);
        assert_eq!(bus.tvram_wait(false), 0);
        bus.mem_wait_on = false;
        assert_eq!(bus.tvram_wait(true), 0);

        let mut bus = bus_8mhz();
        bus.mem_wait_on = false;
        assert_eq!(bus.tvram_wait(true), 2);
        assert_eq!(bus.tvram_wait(false), 1);
        // The memory-wait switch does not change the 8 MHz TVRAM result.
        bus.mem_wait_on = true;
        assert_eq!(bus.tvram_wait(true), 2);
        assert_eq!(bus.tvram_wait(false), 1);
    }

    #[test]
    fn dictionary_wait_follows_switches() {
        let mut bus = bus_4mhz();
        bus.mem_wait_on = false;
        assert_eq!(bus.dictionary_wait(), 0);
        bus.mem_wait_on = true;
        assert_eq!(bus.dictionary_wait(), 1);

        let mut bus = bus_8mhz();
        bus.mem_wait_on = false;
        assert_eq!(bus.dictionary_wait(), 2);
    }

    #[test]
    fn gvram_wait_graphics_off_follows_memory_wait() {
        let mut bus = bus_4mhz();
        bus.memory.state.gfx_ctrl = 0;
        bus.mem_wait_on = false;
        assert_eq!(bus.gvram_wait(true), 0);
        bus.mem_wait_on = true;
        assert_eq!(bus.gvram_wait(true), 1);
        assert_eq!(bus.gvram_wait(false), 0);
    }

    #[test]
    fn access_wait_dispatches_by_target() {
        let mut bus = bus_8mhz();
        bus.memory.state.gfx_ctrl = 0;
        bus.mem_wait_on = false;
        bus.eight_mhz_fast = false;
        // The ALU and dictionary targets must not use main-RAM timing.
        assert_eq!(bus.dictionary_wait(), 2);
        assert_eq!(bus.access_wait(Pc8801MemoryTarget::DictionaryRom, true), 2);
        // GVRAM ALU with graphics off uses the graphics-off GVRAM wait (3 at
        // 8 MHz), not the main-RAM wait (1 at 8 MHz with these switches).
        assert_eq!(bus.main_wait(true), 1);
        assert_eq!(bus.access_wait(Pc8801MemoryTarget::GvramAlu, true), 3);
    }

    #[test]
    fn m1_wait_only_in_relevant_modes() {
        let mut bus = bus_4mhz();
        bus.mem_wait_on = false;

        // V1S/N/N80, 4 MHz, memory wait off: M1 fetch adds 1 regardless of target.
        for boot_mode in [BootMode::V1S, BootMode::N, BootMode::N80] {
            bus.set_boot_mode(boot_mode);
            assert_eq!(bus.m1_wait(0x8000), 1, "{boot_mode}");
        }

        // V2, 4 MHz: only a TVRAM fetch at 0xF000+ adds 1.
        bus.set_boot_mode(BootMode::V2);
        assert_eq!(bus.m1_wait(0x8000), 0);
        assert_eq!(bus.m1_wait(0xF000), 1);
        assert_eq!(bus.m1_wait(0xE000), 0);

        // N80SR follows the N80 V2 timing: no forced M1 wait.
        bus.set_boot_mode(BootMode::N80SR);
        assert_eq!(bus.m1_wait(0x8000), 0);
        assert_eq!(bus.m1_wait(0xF000), 0);

        // Memory wait on suppresses the M1 wait entirely.
        bus.mem_wait_on = true;
        bus.set_boot_mode(BootMode::V1S);
        assert_eq!(bus.m1_wait(0x8000), 0);
    }

    #[test]
    fn opcode_fetch_adds_m1_wait_but_read_does_not() {
        use common::Bus as _;

        let mut bus = bus_4mhz();
        bus.mem_wait_on = false;
        bus.memory.state.boot_mode = BootMode::V1S;

        // A plain data read of an opcode byte gets no M1 wait.
        let mut view = MainBusView { bus: &mut bus };
        let _ = view.read_byte(0x8000);
        assert_eq!(view.drain_wait_cycles(), 0);

        // The same address fetched as an opcode adds the V1S 4 MHz M1 wait.
        let _ = view.fetch_opcode_byte(0x8000);
        assert_eq!(view.drain_wait_cycles(), 1);
    }

    #[test]
    fn dictionary_reads_use_dictionary_wait_without_gvam() {
        use common::Bus as _;

        let mut bus = bus_8mhz();
        bus.eight_mhz_fast = true;
        bus.mem_wait_on = false;
        bus.memory.state.dic_ctrl = 0x00;

        let mut view = MainBusView { bus: &mut bus };
        let _ = view.read_byte(0xC000);
        assert_eq!(view.drain_wait_cycles(), 2);
    }

    #[test]
    fn dictionary_opcode_fetch_keeps_underlying_tvram_m1_wait() {
        use common::Bus as _;

        let mut bus = bus_4mhz();
        let mut dictionary = vec![0u8; 0x8_0000];
        dictionary[0xF000 - 0xC000] = 0x76;
        bus.memory.load_dictionary_rom(&dictionary);
        bus.mem_wait_on = false;
        bus.memory.state.boot_mode = BootMode::V2;
        bus.memory.state.dic_ctrl = 0x00;

        let mut view = MainBusView { bus: &mut bus };
        assert_eq!(view.fetch_opcode_byte(0xF000), 0x76);
        assert_eq!(view.drain_wait_cycles(), 1);
    }

    #[test]
    fn gvram_accumulator_crosses_limit() {
        let mut bus = bus_4mhz();
        bus.memory.state.gfx_ctrl = GFX_CTRL_GRPHE;

        // 0x1B00 / 0x100 = 27 accesses to cross the 4 MHz limit.
        for _ in 0..26 {
            assert_eq!(bus.insert_gvram_wait(true), 0);
        }
        assert_eq!(bus.insert_gvram_wait(true), 1);

        // Main-RAM accesses never touch the accumulator.
        let before = bus.gvram_access_count;
        let _ = bus.access_wait(Pc8801MemoryTarget::MainRam, true);
        assert_eq!(bus.gvram_access_count, before);
    }

    #[test]
    fn assert_busreq_only_in_standard_modes() {
        let mut bus = bus_4mhz();
        bus.busreq_clocks = 500;

        for boot_mode in [BootMode::V2, BootMode::V1H] {
            bus.busreq_until = 0;
            bus.set_boot_mode(boot_mode);
            bus.assert_busreq(1_000);
            assert_eq!(
                bus.busreq_until, 0,
                "{boot_mode} never locks the CPU off the bus"
            );
        }

        for boot_mode in [BootMode::V1S, BootMode::N, BootMode::N80, BootMode::N80SR] {
            bus.busreq_until = 0;
            bus.set_boot_mode(boot_mode);
            bus.assert_busreq(1_000);
            assert_eq!(bus.busreq_until, 1_500, "{boot_mode} asserts BUSREQ");
            bus.current_cycle = 1_200;
            assert!(bus.busreq_active());
            bus.current_cycle = 1_500;
            assert!(!bus.busreq_active());
        }
    }

    #[test]
    fn phase_ports_have_reference_readback_defaults() {
        let mut bus = bus_4mhz();

        assert_eq!(bus.io_read(0x00).0, 0xFF);
        assert_eq!(bus.io_read(0x0D).0, 0xFF);
        assert_eq!(bus.io_read(0x0E).0, 0x7F);
        assert_eq!(bus.io_read(0x0F).0, 0xFF);
        bus.keyboard_rows[2] = 0b1111_1101;
        assert_eq!(bus.io_read(0x02).0, 0b1111_1101);

        assert_eq!(bus.io_read(0x30).0, 0xFB);
        assert_eq!(bus.io_read(0x31).0, 0x79);

        bus.io_write(0x32, 0x95);
        assert_eq!(bus.io_read(0x32).0, 0x95);

        // No disk is mounted, so the BOOT line (bit 3) is asserted: the reset code
        // boots the N88-BASIC ROM rather than the disk system.
        assert_eq!(bus.io_read(0x40).0 & 0xDF, 0xCF);
        bus.vrtc_active = true;
        assert_eq!(bus.io_read(0x40).0, 0xEF);
    }

    #[test]
    fn n_family_port30_distinguishes_pc8001_personalities() {
        let mut bus = bus_4mhz();

        for (boot_mode, expected) in [
            (BootMode::N, 0x02),
            (BootMode::N80, 0x03),
            (BootMode::N80SR, 0x01),
        ] {
            bus.set_boot_mode(boot_mode);
            assert_eq!(bus.io_read(0x30).0 & 0x03, expected, "{boot_mode}");
        }
    }

    #[test]
    fn n80_port33_controls_compatible_misc_bits() {
        let mut bus = bus_4mhz();

        assert_eq!(bus.io_read(0x33).0, OPEN_BUS);

        bus.set_boot_mode(BootMode::N80);
        assert_eq!(bus.io_read(0x33).0, 0x00);
        bus.io_write(0x32, 0x40);
        assert_eq!(bus.io_read(0x32).0, OPEN_BUS);
        assert_eq!(bus.memory.state.misc_ctrl & MISC_CTRL_GVAM, 0x00);

        bus.io_write(0x33, N80_CTRL_GVAM | N80_CTRL_SINTM | 0x04 | 0x01 | 0x20);
        assert_eq!(bus.io_read(0x33).0, N80_CTRL_GVAM | N80_CTRL_SINTM | 0x04);
        assert_eq!(bus.memory.state.misc_ctrl & MISC_CTRL_GVAM, MISC_CTRL_GVAM);
        assert_eq!(
            bus.memory.state.misc_ctrl & MISC_CTRL_SINTM,
            MISC_CTRL_SINTM
        );

        bus.io_write(0x33, 0x00);
        assert_eq!(bus.io_read(0x33).0, 0x00);
        assert_eq!(bus.memory.state.misc_ctrl & MISC_CTRL_GVAM, 0x00);
        assert_eq!(bus.memory.state.misc_ctrl & MISC_CTRL_SINTM, 0x00);
    }

    #[test]
    fn n80sr_boot_mode_sets_port33_rom_select_bit() {
        let mut bus = bus_4mhz();

        bus.set_boot_mode(BootMode::N80SR);

        assert_eq!(bus.io_read(0x33).0 & N80_CTRL_N80SR, N80_CTRL_N80SR);
        assert_eq!(bus.io_read(0x31).0 & 0xC0, 0xC0);
        bus.io_write(0x53, 0x0F);
        assert_eq!(bus.layer_disable(), 0x00);
    }

    #[test]
    fn bank_and_machine_control_ports_read_back_latches() {
        let mut bus = bus_4mhz();

        bus.io_write(0x5C, 0);
        assert_eq!(bus.io_read(0x5C).0, 0xF9);
        bus.io_write(0x5D, 0);
        assert_eq!(bus.io_read(0x5C).0, 0xFA);
        bus.io_write(0x5E, 0);
        assert_eq!(bus.io_read(0x5C).0, 0xFC);
        bus.io_write(0x5F, 0);
        assert_eq!(bus.io_read(0x5C).0, 0xF8);

        assert_eq!(bus.io_read(0x6E).0, 0x90);
        bus.io_write(0x6F, 0x0D);
        assert_eq!(bus.io_read(0x6F).0, 0xFD);

        bus.io_write(0x70, 0xF3);
        bus.io_write(0x71, 0x01);
        assert_eq!(bus.io_read(0x70).0, 0xF3);
        assert_eq!(bus.io_read(0x71).0, 0x01);

        assert_eq!(bus.io_read(0xE2).0, 0xFF);
        bus.io_write(0xE2, 0x11);
        bus.io_write(0xE3, 0x02);
        assert_eq!(bus.io_read(0xE2).0, 0xEE);
        assert_eq!(bus.io_read(0xE3).0, 0xF2);
    }

    #[test]
    fn cpu_clock_port_reflects_fast_clock_position() {
        let mut bus = bus_8mhz();

        assert_eq!(bus.io_read(0x6E).0, 0x10);
    }

    #[test]
    fn set_key_drives_the_active_low_matrix() {
        let mut bus = bus_4mhz();

        // 'A' sits at matrix row 2, column 1.
        bus.set_key(2, 1, true);
        assert_eq!(bus.io_read(0x02).0, 0b1111_1101);

        bus.set_key(2, 1, false);
        assert_eq!(bus.io_read(0x02).0, 0xFF);
    }

    #[test]
    fn mouse_buttons_read_active_low_on_the_ssg_port() {
        let mut bus = bus_4mhz();
        bus.set_mouse_buttons(true, false);

        // Select SSG port B (register 0x0F) and read it through OPN data port.
        bus.io_write(0x44, 0x0F);
        let port_b = bus.io_read(0x45).0;
        assert_eq!(port_b & 0x01, 0, "left button reads active low");
        assert_eq!(port_b & 0x02, 0x02, "right button stays released");
    }

    #[test]
    fn mouse_strobe_shifts_the_movement_nibbles() {
        let mut bus = bus_4mhz();
        // Accumulated movement; the latched delta is the negated movement.
        bus.set_mouse_delta(0x12, 0x34);

        // Each port 0x40 bit 6 edge advances the readout phase. The first edge
        // (from the idle phase) latches and presents the X high nibble.
        bus.io_write(0x44, 0x0E); // select SSG port A
        // Latched delta is 0xEECC (-0x12 = 0xEE, -0x34 = 0xCC); each nibble is
        // returned in the low four bits with the high nibble forced to 1.
        let expected = [0xFE_u8, 0xFE, 0xFC, 0xFC];
        let mut strobe = 0u8;
        for expected_value in expected {
            strobe ^= PORT40_MOUSE_STROBE;
            bus.io_write(0x40, strobe);
            assert_eq!(bus.io_read(0x45).0, expected_value);
        }
    }

    #[test]
    fn joystick_directions_read_active_low_on_port_a() {
        let mut bus = bus_4mhz();
        bus.set_joystick(JoystickState {
            up: true,
            right: true,
            ..JoystickState::default()
        });

        bus.io_write(0x44, 0x0E); // select SSG port A
        let port_a = bus.io_read(0x45).0;
        assert_eq!(port_a & 0x01, 0, "up reads active low");
        assert_eq!(port_a & 0x08, 0, "right reads active low");
        assert_eq!(port_a & 0x02, 0x02, "down stays released");
        assert_eq!(port_a & 0x04, 0x04, "left stays released");
        assert_eq!(port_a & 0xF0, 0xF0, "the high nibble is forced to 1");
    }

    #[test]
    fn joystick_triggers_combine_with_mouse_buttons_on_port_b() {
        let mut bus = bus_4mhz();
        bus.set_joystick(JoystickState {
            trigger1: true,
            ..JoystickState::default()
        });

        bus.io_write(0x44, 0x0F); // select SSG port B
        let port_b = bus.io_read(0x45).0;
        assert_eq!(port_b & 0x01, 0, "trigger 1 reads active low");
        assert_eq!(port_b & 0x02, 0x02, "trigger 2 stays released");
    }

    #[test]
    fn mouse_strobe_takes_port_a_then_joystick_reclaims_it() {
        let mut bus = bus_4mhz();
        // With no strobe yet, the joystick owns port A.
        bus.set_joystick(JoystickState {
            left: true,
            ..JoystickState::default()
        });
        bus.io_write(0x44, 0x0E);
        assert_eq!(
            bus.io_read(0x45).0 & 0x04,
            0,
            "joystick left visible before any strobe"
        );

        // A mouse strobe sequence takes over port A with the movement nibble.
        bus.set_mouse_delta(0x12, 0x34);
        bus.io_write(0x40, PORT40_MOUSE_STROBE);
        assert_eq!(
            bus.io_read(0x45).0,
            0xFE,
            "mouse owns port A while strobing"
        );

        // Once the strobe times out, the joystick reclaims port A.
        bus.set_current_cycle(bus.mouse_timeout_cycles + 10);
        bus.set_joystick(JoystickState {
            left: true,
            ..JoystickState::default()
        });
        assert_eq!(
            bus.io_read(0x45).0 & 0x04,
            0,
            "joystick reclaims port A after timeout"
        );
    }

    #[test]
    fn monitor_timing_follows_selection_and_line_mode() {
        let mut bus = bus_4mhz();
        let period_15k = u64::from(bus.clocks.main_clock_hz) / HORIZONTAL_FREQ_15KHZ;
        let period_24k = u64::from(bus.clocks.main_clock_hz) / HORIZONTAL_FREQ_24KHZ;

        // Auto mode: 200-line text (gfx_ctrl bit 0 set) stays 15 kHz.
        bus.io_write(0x31, 0x01);
        assert!(!bus.monitor_is_hireso());
        assert_eq!(bus.crtc_line_period(), period_15k);

        // Selecting 400-line graphics (bits 0 and 4 clear) switches to 24 kHz.
        bus.io_write(0x31, 0x08);
        assert!(bus.monitor_is_hireso());
        assert_eq!(bus.crtc_line_period(), period_24k);

        // Forcing 15 kHz overrides the line mode.
        bus.set_monitor_timing(MonitorTiming::Fixed15kHz);
        assert!(!bus.monitor_is_hireso());
        bus.io_write(0x31, 0x08);
        assert!(
            !bus.monitor_is_hireso(),
            "fixed 15 kHz ignores the line mode"
        );

        // Forcing 24 kHz.
        bus.set_monitor_timing(MonitorTiming::Fixed24kHz);
        assert!(bus.monitor_is_hireso());
        assert_eq!(bus.crtc_line_period(), period_24k);
    }

    /// Drives the CD-ROM SCSI READ(6) path the way the CD-System BIOS does when
    /// auto-booting a data disc: program DMA channel 1, enable the drive and DMA,
    /// select the target, then clock the command bytes through port 0x91. The
    /// sector data must land in main RAM via the channel-1 DMA transfer.
    #[test]
    fn cdrom_read_transfers_sector_to_ram_over_dma() {
        let mut bus = bus_4mhz();

        // Single MODE1/2048 data track whose first sector holds a known pattern.
        let cue = "FILE \"test.bin\" BINARY\n  TRACK 01 MODE1/2048\n    INDEX 01 00:00:00\n";
        let mut bin = vec![0u8; 2048 * 2];
        for (index, byte) in bin[..2048].iter_mut().enumerate() {
            *byte = (index & 0xFF) as u8;
        }
        let image = CdImage::from_cue(cue, bin).expect("cue parses");
        bus.insert_cdrom(image);

        // Program DMA channel 1 (ports 0x62/0x63): destination 0xA000, count =
        // 2048 bytes (the i8257 transfers count + 1, so program 2047 = 0x07FF).
        const CDROM_CHANNEL: usize = 1;
        let address_port = 0x60 + ((CDROM_CHANNEL as u16) << 1); // 0x62.
        let count_port = address_port | 1; // 0x63.
        bus.io_write(address_port, 0x00);
        bus.io_write(address_port, 0xA0);
        bus.io_write(count_port, 0xFF);
        bus.io_write(count_port, 0x07);
        bus.io_write(0x68, 1 << CDROM_CHANNEL); // Mode register: enable channel 1.

        // Enable the drive (bit 0) and DMA (bit 6) via port 0x9F.
        bus.io_write(0x9F, 0x41);

        // SCSI selection: assert then deassert SEL.
        bus.io_write(0x90, 0x01);
        bus.io_write(0x90, 0x00);

        // READ(6): one block at LBA 0. The final byte completes the CDB, runs the
        // command, and drains the DMA transfer into RAM.
        for byte in [0x08u8, 0x00, 0x00, 0x00, 0x01, 0x00] {
            bus.io_write(0x91, byte);
        }

        for index in 0..2048 {
            assert_eq!(
                bus.memory.state.ram[0xA000 + index],
                (index & 0xFF) as u8,
                "sector byte {index} should be DMA'd into RAM"
            );
        }

        // The transfer finished, so the controller is in the status phase and
        // reports GOOD on the data port.
        assert_eq!(bus.io_read(0x91).0, 0x00, "SCSI status should be GOOD");

        // Port 0x99 identifies the board as a CD-ROM-equipped MC.
        assert_eq!(bus.io_read(0x99).0, 0xCD);
    }
}
