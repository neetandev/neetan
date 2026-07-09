//! FM Towns system bus.
//!
//! Owns the memory map and the core chipset (dual 8259 PICs, the interval
//! timer, the two uPD71071 DMA controllers, the MSM58321 RTC, and the serial
//! keyboard), and dispatches the CPU's memory and I/O accesses. The FM Towns
//! decodes the full 16-bit I/O port address, so the dispatch matches on the
//! whole port value.

mod elevol;
mod gameport;
mod io_read;
mod io_write;
mod keyboard;
mod sprite;
mod video;

use std::path::PathBuf;

use common::{BeeperKind, Bus, NoTracing, Tracing};
use device::{
    beeper::Beeper,
    cdrom_towns::TownsCdController,
    disk::{HddImage, MountedHdd},
    floppy::{FloppyImage, MountedFloppy},
    i8251_serial::I8251Serial,
    i8259a_pic::I8259aPic,
    mb8877_fdc::{Mb8877Config, Mb8877Fdc},
    msm58321_rtc::Msm58321Rtc,
    opn_fm::{FmTimerAction, OpnFm, Ymf276},
    rf5c68::Rf5c68,
    scsi::{ScsiDmaRequest, TownsScsiController},
    upd71071_dma::Upd71071Dma,
};
use elevol::ElectronicVolume;
use gameport::TownsGamePort;
use keyboard::TownsKeyboard;
use software_renderer::{RenderInputsTowns, TownsRenderer};
use sprite::TownsSprite;
use video::TownsVideo;

use crate::{
    config::{ClockConfig, TownsModel},
    memory::TownsMemory,
    scheduler::{EventTowns, TownsScheduler},
    timer::{TIMER_CLOCK_HZ, TownsTimer},
};

/// FM Towns VSYNC interrupt line into the slave PIC (slave IR3 = IRQ 11).
const IRQ_VSYNC: u8 = 11;

/// FM Towns CD-ROM interrupt line into the slave PIC (slave IR1 = IRQ 9).
const IRQ_CDROM: u8 = 9;

/// FM Towns built-in RS-232C interrupt line into the master PIC (IRQ 2).
const IRQ_RS232C: u8 = 2;

/// RS-232C interrupt-enable bit (I/O 0x0A08) for the receiver-ready source.
const RS232C_INT_ENABLE_RXRDY: u8 = 0x02;

/// FM Towns sound interrupt line into the slave PIC (slave IR5 = IRQ 13). The
/// OPN2 FM timers and the RF5C68 PCM chip share it.
const IRQ_SOUND: u8 = 13;

/// YM3438/YMF276 (OPN2) input clock: 8 MHz (internal FM clock 8 MHz / 12).
const OPN2_INPUT_CLOCK_HZ: u32 = 8_000_000;

/// Sound mute latch bit gating the RF5C68 PCM output.
const MUTE_PCM_AUDIBLE: u8 = 0x01;
/// Sound mute latch bit gating the OPN2 FM output.
const MUTE_FM_AUDIBLE: u8 = 0x02;
/// Audio-out latch bit enabling the speaker/line output as a whole.
const AUDIO_OUT_ENABLE: u8 = 0x40;

/// The electronic-volume chip whose channels 0/1 attenuate CD-DA left/right.
const ELEVOL_CD: usize = 1;
const ELEVOL_CD_LEFT: usize = 0;
const ELEVOL_CD_RIGHT: usize = 1;

/// Memory-mapped FMR buzzer control (0xCFF98): reading it turns the beep on,
/// writing it turns the beep off. Independent of the SOUND enable bit at I/O
/// 0x0060; either source gates the same buzzer.
const TOWNSMEMIO_BUZZER_CONTROL: u32 = 0x000C_FF98;

/// Physical base of the RF5C68 wave-RAM window (4 KB, banked).
const RF5C68_WAVE_WINDOW_BASE: u32 = 0xC220_0000;
/// One past the last byte of the RF5C68 wave-RAM window.
const RF5C68_WAVE_WINDOW_END: u32 = 0xC220_1000;

/// The CD-ROM controller drives DMA channel 3 of the main uPD71071.
const CDROM_DMA_CHANNEL: usize = 3;

/// FM Towns FDC interrupt line into the master PIC (IRQ 6).
const IRQ_FDC: u8 = 6;

/// The MB8877 FDC drives DMA channel 0 of the main uPD71071.
const FDC_DMA_CHANNEL: usize = 0;

/// FM Towns SCSI interrupt line into the slave PIC (slave IR0 = IRQ 8).
const IRQ_SCSI: u8 = 8;

/// The SCSI SPC drives DMA channel 1 of the main uPD71071.
const SCSI_DMA_CHANNEL: usize = 1;

/// Memory waits programmed by a FASTMODE (I/O 0x05EC) write with bit 0 clear.
const SLOW_MODE_MEMORY_WAITS: u8 = 6;

/// The FASTMODE lamp reads lit while the VRAM wait stays below this value.
const FAST_MODE_LAMP_VRAM_WAIT_LIMIT: u8 = 3;

/// Vertical-sync pulse duration in microseconds (measured ~60 us on a real MX).
const VSYNC_DURATION_MICROS: u64 = 60;

/// Scanline period in nanoseconds (~31 kHz horizontal frequency).
const SCANLINE_DURATION_NANOS: u64 = 32_768;

/// Displayed portion of a scanline in nanoseconds; the remainder up to
/// [`SCANLINE_DURATION_NANOS`] is the horizontal blanking window.
const SCANLINE_DISPLAY_NANOS: u64 = 30_000;

/// Number of microseconds in one second, for sub-second time derivation.
const MICROS_PER_SECOND: u64 = 1_000_000;

/// Number of nanoseconds in one second, for scanline timing derivation.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// The two DMA controllers: the main uPD71071 and the second "EXDMAC" bank.
const DMA_MAIN: usize = 0;
const DMA_EXTENDED: usize = 1;

/// Default host time source (a fixed timestamp) until the app installs one.
fn default_host_local_time() -> [u8; 6] {
    // 2000-01-01 (Saturday) 00:00:00, BCD, `[year, month<<4|weekday, day, hour,
    // minute, second]`.
    [0x00, 0x16, 0x01, 0x00, 0x00, 0x00]
}

/// The FM Towns system bus.
pub struct TownsBus<T: Tracing = NoTracing> {
    pub(crate) memory: TownsMemory,
    pub(crate) clocks: ClockConfig,
    pub(crate) current_cycle: u64,
    pub(crate) next_event_cycle: u64,
    pub(crate) scheduler: TownsScheduler,
    pub(crate) pic: I8259aPic,
    pub(crate) timer: TownsTimer,
    pub(crate) dmac: [Upd71071Dma; 2],
    pub(crate) rtc: Msm58321Rtc,
    pub(crate) keyboard: TownsKeyboard,
    pub(crate) cdc: TownsCdController,
    /// MB8877 floppy disk controller at I/O 0x0200-0x020E (IRQ 6, DMA channel 0).
    pub(crate) fdc: Mb8877Fdc,
    /// MB89352-class SCSI controller at I/O 0x0C30-0x0C37 (IRQ 8, DMA channel 1).
    pub(crate) scsi: TownsScsiController,
    /// PC-speaker-style buzzer. Its tone follows interval-timer channel 2 and it
    /// is gated by the SOUND enable bit (I/O 0x0060) or the memory-mapped buzzer
    /// control (0xCFF98).
    pub(crate) beeper: Beeper,
    /// Latched state of the memory-mapped buzzer control (0xCFF98): true while a
    /// read has turned the beep on and no write has turned it off.
    pub(crate) buzzer_memio: bool,
    /// OPN2 FM sound chip (YMF276/YM3438 class) at I/O 0x04D8-0x04DE.
    pub(crate) fm: OpnFm<Ymf276>,
    /// RF5C68 8-channel PCM chip at I/O 0x04F0-0x04F8, wave RAM at 0xC2200000.
    pub(crate) pcm: Rf5c68,
    /// Sound mute latch (I/O 0x04D5): bit 1 gates the FM output, bit 0 the PCM
    /// output.
    pub(crate) sound_mute: u8,
    /// Audio-out latch (I/O 0x04EC): bit 6 is the master output enable gating
    /// FM, PCM, and CD-DA.
    pub(crate) sound_audio: u8,
    /// Electronic-volume attenuators (I/O 0x04E0-0x04E3); the second chip's
    /// channels 0/1 set the CD-DA left/right level.
    pub(crate) elevol: [ElectronicVolume; 2],
    /// Main-RAM wait latch (I/O 0x05E0 first-generation alias / 0x05E2); stored
    /// and read back only, the slow-mode clock change is not modeled.
    pub(crate) main_ram_wait: u8,
    /// VRAM wait latch (I/O 0x05E6); stored and read back only.
    pub(crate) vram_wait: u8,
    pub(crate) gameport: TownsGamePort,
    pub(crate) video: TownsVideo,
    pub(crate) sprite: TownsSprite,
    pub(crate) renderer: TownsRenderer,
    /// Valid display width from the last composed frame.
    pub(crate) display_width: u32,
    /// Valid display height from the last composed frame.
    pub(crate) display_height: u32,
    /// The machine model this bus was built for.
    pub(crate) model: TownsModel,
    /// Machine identity bytes for I/O 0x0030 (low) and 0x0031 (high).
    pub(crate) machine_id: (u8, u8),
    /// Cycle of the most recent vertical-sync start edge, anchoring the raster
    /// position within the current frame.
    pub(crate) last_vsync_start_cycle: u64,
    /// NMI mask register latch (I/O 0x0028).
    pub(crate) nmi_mask: u8,
    /// Built-in RS-232C USART (I/O 0x0A00-0x0A08), IRQ 2.
    pub(crate) rs232c: I8251Serial,
    /// RS-232C interrupt-enable latch (I/O 0x0A08): bit 0 TxRDY, bit 1 RxRDY,
    /// bit 2 SYNDET.
    pub(crate) rs232c_int_enable: u8,
    /// Reset-reason latch (I/O 0x0020): bit 0 set after a software reset,
    /// cleared when the port is read.
    pub(crate) reset_reason: u8,
    /// A soft reset was requested through I/O 0x0020/0x0022 and is pending.
    pub(crate) soft_reset_pending: bool,
    /// A power-off was requested through I/O 0x0020/0x0022.
    pub(crate) power_off_requested: bool,
    /// Bit position of the next serial machine-ID EEPROM bit (I/O 0x0032).
    pub(crate) serial_rom_bit_count: u8,
    /// Last value written to the serial machine-ID EEPROM port (I/O 0x0032),
    /// for edge detection on the clock and ID-reset lines.
    pub(crate) last_serial_rom_command: u8,
    /// Selected memory-card bank (I/O 0x0490).
    pub(crate) memcard_bank: u8,
    /// Memory-card attribute register-select latch (I/O 0x0491 bit 0).
    pub(crate) memcard_reg: bool,
    /// Host local-time source (BCD) for the RTC.
    pub(crate) host_local_time_fn: fn() -> [u8; 6],
    /// Roland MT-32 sound module, fed by RS-MIDI bytes (optional, requires munt).
    #[cfg(feature = "mt32")]
    mt32: Option<device::mt32::Mt32>,
    /// Roland SC-55 sound module, fed by RS-MIDI bytes (optional, requires Nuked-SC55).
    #[cfg(feature = "sc55")]
    sc55: Option<device::sc55::Sc55>,
    pub(crate) tracer: T,
}

impl<T: Tracing + Default> TownsBus<T> {
    /// Builds the bus over a prepared memory map and clock configuration.
    pub(crate) fn new(memory: TownsMemory, clocks: ClockConfig, model: TownsModel) -> Self {
        let mut bus = Self {
            memory,
            clocks,
            current_cycle: 0,
            next_event_cycle: u64::MAX,
            scheduler: TownsScheduler::new(),
            // The SYSROM programs the PICs; start from a cleared state.
            pic: I8259aPic::new_zeroed(),
            timer: TownsTimer::new(),
            dmac: [Upd71071Dma::new(), Upd71071Dma::new()],
            rtc: Msm58321Rtc::new(),
            keyboard: TownsKeyboard::new(),
            cdc: TownsCdController::new(clocks.sample_rate, clocks.cpu_clock_hz),
            fdc: Mb8877Fdc::new(clocks.cpu_clock_hz, Mb8877Config::towns()),
            scsi: TownsScsiController::new(clocks.cpu_clock_hz),
            beeper: Beeper::new(BeeperKind::PitDriven, TIMER_CLOCK_HZ),
            buzzer_memio: false,
            fm: OpnFm::new(clocks.cpu_clock_hz, clocks.sample_rate, OPN2_INPUT_CLOCK_HZ),
            pcm: Rf5c68::new(clocks.sample_rate),
            sound_mute: 0,
            sound_audio: 0,
            elevol: [ElectronicVolume::new(), ElectronicVolume::new()],
            main_ram_wait: 0,
            vram_wait: 0,
            gameport: TownsGamePort::new(clocks.cpu_clock_hz),
            video: TownsVideo::new(model.high_res_available()),
            sprite: TownsSprite::new(clocks.cpu_clock_hz),
            renderer: TownsRenderer::new(),
            display_width: 640,
            display_height: 480,
            model,
            machine_id: model.machine_id(),
            last_vsync_start_cycle: 0,
            nmi_mask: 0,
            rs232c: I8251Serial::new(),
            rs232c_int_enable: 0,
            reset_reason: 0,
            soft_reset_pending: false,
            power_off_requested: false,
            serial_rom_bit_count: 0,
            last_serial_rom_command: 0,
            memcard_bank: 0,
            memcard_reg: false,
            host_local_time_fn: default_host_local_time,
            #[cfg(feature = "mt32")]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: None,
            tracer: T::default(),
        };
        bus.schedule_next_vsync();
        bus
    }
}

impl<T: Tracing> TownsBus<T> {
    /// Overrides the host local-time source (BCD) used by the RTC.
    pub(crate) fn set_host_local_time_fn(&mut self, host_local_time_fn: fn() -> [u8; 6]) {
        self.host_local_time_fn = host_local_time_fn;
    }

    /// Installs a Roland MT-32 sound module driven by RS-MIDI (RS-232C) output.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.mt32 = Some(device::mt32::Mt32::new(rom_directory)?);
        self.rs232c.enable_midi_capture();
        Ok(())
    }

    /// Installs a Roland SC-55 sound module driven by RS-MIDI (RS-232C) output.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.sc55 = Some(device::sc55::Sc55::new(rom_directory)?);
        self.rs232c.enable_midi_capture();
        Ok(())
    }

    /// Enables RS-232C MIDI transmit capture without installing a sound module.
    /// Used to exercise the RS-MIDI path without loading ROMs.
    pub fn enable_midi_capture(&mut self) {
        self.rs232c.enable_midi_capture();
    }

    /// Drains the RS-232C MIDI transmit buffer into `out`.
    pub fn flush_midi_into(&mut self, out: &mut Vec<u8>) {
        self.rs232c.flush_midi_into(out);
    }

    /// Injects a byte into the RS-232C receiver as if it arrived on the serial
    /// line, refreshing the interrupt. Used to exercise the receive path in
    /// tests without external hardware.
    pub fn push_rs232c_received_byte(&mut self, byte: u8) {
        self.rs232c.push_received_byte(byte);
        self.refresh_rs232c_irq();
    }

    /// Queues a keyboard scancode from the host and refreshes IRQ 1.
    pub(crate) fn push_key_scancode(&mut self, code: u8) {
        self.keyboard.push_scancode(code);
        self.refresh_keyboard_irq();
    }

    /// Accumulates a relative mouse movement from the host.
    pub(crate) fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.gameport.push_mouse_delta(dx, dy);
    }

    /// Updates the mouse button state.
    pub(crate) fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.gameport.set_mouse_buttons(left, right);
    }

    /// Updates the game pad direction and button state on port 0.
    pub(crate) fn set_pad(&mut self, state: common::JoystickState) {
        self.gameport.set_pad(state);
    }

    /// Selects the pad type on game port 0.
    pub(crate) fn set_pad_type(&mut self, kind: crate::config::TownsPadType) {
        self.gameport.set_pad_type(kind);
    }

    /// The FONT ROM image, exposed to the app's image selector.
    pub(crate) fn font_rom_data(&self) -> &[u8] {
        self.memory.font_rom()
    }

    /// Whether a power-off has been requested through I/O 0x0020/0x0022.
    pub(crate) fn power_off_requested(&self) -> bool {
        self.power_off_requested
    }

    /// Consumes a pending soft-reset request, returning whether one was set.
    pub(crate) fn take_soft_reset(&mut self) -> bool {
        let pending = self.soft_reset_pending;
        self.soft_reset_pending = false;
        pending
    }

    /// Read-only view of the VRAM, for tests and debugging tools.
    pub fn vram(&self) -> &[u8] {
        self.memory.vram()
    }

    /// Returns a reference to the tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Returns a mutable reference to the tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// The cycle of the next scheduled event, if any.
    pub(crate) fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Microseconds elapsed within the current second, for the RTC ready flag.
    fn subsecond_micros(&self) -> u32 {
        let micros = self.current_cycle * MICROS_PER_SECOND / u64::from(self.clocks.cpu_clock_hz);
        (micros % MICROS_PER_SECOND) as u32
    }

    /// The free-running 1 MHz microsecond counter (I/O 0x0026-0x0027).
    fn free_run_counter(&self) -> u16 {
        let micros = self.current_cycle * MICROS_PER_SECOND / u64::from(self.clocks.cpu_clock_hz);
        micros as u16
    }

    /// Recomputes the buzzer gate from its two sources (the SOUND enable bit and
    /// the memory-mapped buzzer latch) and records the transition for the beeper.
    fn refresh_beeper_gate(&mut self) {
        let enabled = self.timer.sound_enabled() || self.buzzer_memio;
        self.beeper.set_buzzer_enabled(enabled, self.current_cycle);
    }

    /// Reasserts or clears the timer IRQ 0 line into the master PIC.
    fn refresh_timer_irq(&mut self) {
        if self.timer.irq_active() {
            self.pic.set_irq(0);
            self.tracer.trace_irq_raise(0);
        } else {
            self.pic.clear_irq(0);
        }
    }

    /// Reasserts or clears the VSYNC IRQ 11 line (slave IR3) into the PIC.
    fn refresh_vsync_irq(&mut self) {
        if self.video.vsync_irq_pending() {
            self.pic.set_irq(IRQ_VSYNC);
            self.tracer.trace_irq_raise(IRQ_VSYNC);
        } else {
            self.pic.clear_irq(IRQ_VSYNC);
        }
    }

    /// Whether the CRTC is inside the horizontal blanking window of the
    /// current scanline. Raster-effect code polls the FR register's DSPTH bits
    /// for this edge, so it must toggle once per scanline during the vertical
    /// display period.
    pub(crate) fn hsync_active(&self) -> bool {
        let cpu_clock_hz = u64::from(self.clocks.cpu_clock_hz);
        let scanline_cycles = (cpu_clock_hz * SCANLINE_DURATION_NANOS / NANOS_PER_SECOND).max(1);
        let display_cycles = cpu_clock_hz * SCANLINE_DISPLAY_NANOS / NANOS_PER_SECOND;
        (self.current_cycle % scanline_cycles) >= display_cycles
    }

    /// The per-layer vertical display state at the current raster position,
    /// measured from the most recent vertical-sync start edge.
    pub(crate) fn vertical_display_active(&self) -> (bool, bool) {
        let frame_cycles = self.video.frame_cycles(self.clocks.cpu_clock_hz).max(1);
        let into_frame = self
            .current_cycle
            .saturating_sub(self.last_vsync_start_cycle)
            % frame_cycles;
        self.video.vertical_display_active(into_frame, frame_cycles)
    }

    /// Schedules the next VSYNC start edge one frame period ahead.
    fn schedule_next_vsync(&mut self) {
        let period = self.video.frame_cycles(self.clocks.cpu_clock_hz).max(1);
        self.scheduler.schedule(
            EventTowns::VsyncStart,
            self.current_cycle.saturating_add(period),
        );
        self.update_next_event_cycle();
    }

    /// The composed RGBA framebuffer from the last rendered frame.
    pub(crate) fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// The valid `(width, height)` of the composed framebuffer.
    pub(crate) fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
    }

    /// Composes one frame into the renderer's framebuffer from CRTC state.
    fn render_frame(&mut self) {
        let display_planes = self.memory.fmr_display_planes();
        let fmr_display_page_offset = self.memory.fmr_display_page_offset();
        let sprite_display_offset = self.sprite.display_vram_offset();
        let resolved = self.video.resolve(
            display_planes,
            fmr_display_page_offset,
            sprite_display_offset,
        );
        let inputs = RenderInputsTowns {
            vram: self.memory.vram(),
            single_page: resolved.single_page,
            priority_page: resolved.priority_page,
            layers: resolved.layers,
            palette_16: resolved.palette_16,
            palette_256: resolved.palette_256,
            width: resolved.width,
            height: resolved.height,
            high_res: resolved.high_res,
            mouse_cursor: resolved.mouse_cursor,
        };
        let (width, height) = self.renderer.render(&inputs);
        self.display_width = width;
        self.display_height = height;
    }

    /// Reasserts or clears the RS-232C IRQ 2 line into the master PIC. The
    /// receiver-ready source is gated by its interrupt-enable bit (0x0A08).
    fn refresh_rs232c_irq(&mut self) {
        let rx_int = self.rs232c_int_enable & RS232C_INT_ENABLE_RXRDY != 0
            && self.rs232c.read_status() & 0x02 != 0;
        if rx_int {
            self.pic.set_irq(IRQ_RS232C);
            self.tracer.trace_irq_raise(IRQ_RS232C);
        } else {
            self.pic.clear_irq(IRQ_RS232C);
        }
    }

    /// The RS-232C interrupt-reason byte (I/O 0x0A06): the upper bits float high
    /// and bit 0 reflects an active, enabled interrupt source.
    pub(crate) fn rs232c_int_reason(&self) -> u8 {
        let rx_int = self.rs232c_int_enable & RS232C_INT_ENABLE_RXRDY != 0
            && self.rs232c.read_status() & 0x02 != 0;
        0xF8 | u8::from(rx_int)
    }

    /// Advances the serial machine-ID EEPROM state on a write to I/O 0x0032.
    /// Bit 5 is chip-select (active low), bit 6 is ID-reset, bit 7 is the clock.
    pub(crate) fn write_serial_rom(&mut self, value: u8) {
        let last = self.last_serial_rom_command;
        let chip_selected = value & 0x20 == 0;
        if chip_selected && last & 0x80 != 0 && value & 0x80 == 0 {
            // Falling clock edge while selected: restart the bit sequence.
            self.serial_rom_bit_count = 0;
        } else if value & 0xA0 == 0 && last & 0x40 == 0 && value & 0x40 != 0 {
            // Rising ID-reset edge while selected: advance to the next bit.
            self.serial_rom_bit_count = self.serial_rom_bit_count.wrapping_add(1);
        }
        self.last_serial_rom_command = value;
    }

    /// Reads one bit of the serial machine-ID EEPROM (I/O 0x0032). The clock and
    /// ID-reset bits echo back in the upper bits; bit 0 carries the ROM bit,
    /// walking the array from the last byte backwards, least-significant bit first.
    pub(crate) fn read_serial_rom(&self) -> u8 {
        let mut data = self.last_serial_rom_command & 0xC0;
        let serial_rom = self.memory.serial_rom();
        if !serial_rom.is_empty() {
            let index = serial_rom.len() - 1 - usize::from(self.serial_rom_bit_count >> 3);
            let bit = 1u8 << (self.serial_rom_bit_count & 7);
            if serial_rom[index] & bit != 0 {
                data |= 0x01;
            }
        }
        data
    }

    /// Reasserts or clears the keyboard IRQ 1 line into the master PIC.
    fn refresh_keyboard_irq(&mut self) {
        if self.keyboard.irq_line() {
            self.pic.set_irq(1);
            self.tracer.trace_irq_raise(1);
        } else {
            self.pic.clear_irq(1);
        }
    }

    /// Reasserts or clears the CD-ROM IRQ 9 line (slave IR1) into the PIC.
    fn refresh_cdrom_irq(&mut self) {
        let (status_irq, data_end_irq) = self.cdc.interrupt_flags();
        self.tracer.trace_cd_irq(status_irq, data_end_irq);
        if self.cdc.irq_line() {
            self.pic.set_irq(IRQ_CDROM);
            self.tracer.trace_irq_raise(IRQ_CDROM);
        } else {
            self.pic.clear_irq(IRQ_CDROM);
        }
    }

    /// Rearms the CD-ROM controller task from its next internal deadline.
    fn reschedule_cdrom(&mut self) {
        match self.cdc.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventTowns::CdTask, cycle),
            None => self.scheduler.cancel(EventTowns::CdTask),
        }
        self.update_next_event_cycle();
    }

    /// True when DMA channel 3 (CD-ROM) is unmasked with a nonzero remaining count.
    fn cdrom_dma_ready(&self) -> bool {
        self.dmac[DMA_MAIN].channel_unmasked(CDROM_DMA_CHANNEL)
            && self.dmac[DMA_MAIN].transfer_length(CDROM_DMA_CHANNEL) > 0
    }

    /// Dispatches a CD-ROM I/O access to the controller and refreshes its IRQ and
    /// scheduled task afterwards.
    fn cdrom_io_read(&mut self, port: u16) -> u8 {
        let value = self.cdc.io_read(port, self.current_cycle);
        if port == 0x04C2 {
            self.tracer.trace_cd_status(&[value]);
        }
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
        value
    }

    fn cdrom_io_write(&mut self, port: u16, value: u8) {
        let dma_ready = self.cdrom_dma_ready();
        self.cdc
            .io_write(port, value, self.current_cycle, dma_ready);
        if port == 0x04C2 {
            self.tracer.trace_cd_command(value, self.cdc.params());
        }
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
    }

    /// Runs the CD-ROM controller task, performing any DMA sector transfer it
    /// requests over channel 3.
    fn service_cdrom_task(&mut self) {
        let dma_ready = self.cdrom_dma_ready();
        let outcome = self.cdc.run_task(self.current_cycle, dma_ready);
        if let Some(sector) = outcome.dma_sector {
            let result = self.dmac[DMA_MAIN].transfer_write_to_memory(CDROM_DMA_CHANNEL, &sector);
            for (address, byte) in result.writes {
                self.memory.write_byte(address, byte);
            }
            // The controller signals ~END to the DMA channel after each sector.
            self.dmac[DMA_MAIN].set_terminal_count(CDROM_DMA_CHANNEL);
            self.cdc.on_dma_transfer_complete(self.current_cycle);
        }
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
    }

    /// Selects the CD-ROM drive timing: the compatibility mode models the real
    /// drive's per-sector and seek delays for games that depend on them.
    pub(crate) fn set_cdrom_compatibility_timing(&mut self, enabled: bool) {
        let mode = if enabled {
            device::cdrom_towns::CdTimingMode::Compatibility {
                drive_speed: self.model.cd_drive_speed(),
            }
        } else {
            device::cdrom_towns::CdTimingMode::Fast
        };
        self.cdc.set_timing_mode(mode);
    }

    /// Inserts a CD-ROM disc image into the drive.
    pub(crate) fn insert_cdrom(&mut self, image: device::cdrom::CdImage) {
        self.cdc.insert(image);
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
    }

    /// Ejects the CD-ROM disc, if any.
    pub(crate) fn eject_cdrom(&mut self) {
        self.cdc.eject();
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
    }

    /// Whether a CD-ROM disc is present.
    pub(crate) fn has_cdrom(&self) -> bool {
        self.cdc.has_disc()
    }

    /// Whether a SCSI hard disk is attached.
    pub(crate) fn has_hdd(&self) -> bool {
        self.scsi.has_drive()
    }

    /// Reasserts or clears the FDC interrupt line (IRQ 6, master PIC).
    fn refresh_fdc_irq(&mut self) {
        if self.fdc.irq_line() {
            self.pic.set_irq(IRQ_FDC);
            self.tracer.trace_irq_raise(IRQ_FDC);
        } else {
            self.pic.clear_irq(IRQ_FDC);
        }
    }

    /// Rearms the FDC command task from its next internal deadline.
    fn reschedule_fdc(&mut self) {
        match self.fdc.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventTowns::FdcTask, cycle),
            None => self.scheduler.cancel(EventTowns::FdcTask),
        }
        self.update_next_event_cycle();
    }

    /// Dispatches an FDC I/O read and refreshes its IRQ and scheduled task. The
    /// low nibble of the port selects the register.
    fn fdc_io_read(&mut self, port: u16) -> u8 {
        let value = match port & 0x0F {
            0x00 => self.fdc.read_status(self.current_cycle),
            0x02 => self.fdc.read_track_register(),
            0x04 => self.fdc.read_sector_register(),
            0x06 => self.fdc.read_data_register(),
            0x08 => self.fdc.read_drive_status(),
            0x0D => 0x7F,
            0x0E => 0xFF,
            _ => 0xFF,
        };
        self.refresh_fdc_irq();
        self.reschedule_fdc();
        value
    }

    fn fdc_io_write(&mut self, port: u16, value: u8) {
        match port & 0x0F {
            0x00 => self.fdc.write_command(value, self.current_cycle),
            0x02 => self.fdc.write_track_register(value),
            0x04 => self.fdc.write_sector_register(value),
            0x06 => self.fdc.write_data_register(value),
            0x08 => self.fdc.write_drive_control(value),
            0x0C => self.fdc.write_drive_select(value),
            _ => {}
        }
        self.refresh_fdc_irq();
        self.reschedule_fdc();
    }

    /// Runs the FDC command task, performing any DMA sector transfer it requests
    /// over channel 0. Read transfers rely on the DMA counter for terminal count;
    /// the FDC is not forced to signal ~END (the CAMELTRY read-sector quirk).
    fn service_fdc_task(&mut self) {
        let outcome = self.fdc.run_task(self.current_cycle);
        if let Some(sector) = outcome.dma_read {
            let result = self.dmac[DMA_MAIN].transfer_write_to_memory(FDC_DMA_CHANNEL, &sector);
            let transferred = result.writes.len();
            for (address, byte) in result.writes {
                self.memory.write_byte(address, byte);
            }
            self.fdc
                .on_read_dma_complete(self.current_cycle, transferred);
        } else if let Some(length) = outcome.dma_write_len {
            let result = self.dmac[DMA_MAIN].transfer_read_from_memory(FDC_DMA_CHANNEL, length);
            let data: Vec<u8> = result
                .addresses
                .iter()
                .map(|&address| self.memory.read_byte(address))
                .collect();
            self.fdc.on_write_dma_complete(self.current_cycle, &data);
        }
        self.refresh_fdc_irq();
        self.reschedule_fdc();
    }

    /// Inserts a mounted floppy into a drive and re-evaluates the FDC IRQ.
    pub(crate) fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: PathBuf) {
        self.fdc
            .insert(drive, MountedFloppy::new(image, Some(path)));
        self.refresh_fdc_irq();
    }

    /// Ejects a drive's floppy, flushing it.
    pub(crate) fn eject_floppy(&mut self, drive: usize) {
        self.fdc.eject(drive);
        self.refresh_fdc_irq();
    }

    /// Flushes all mounted floppies to their backing files.
    pub(crate) fn flush_floppies(&mut self) {
        self.fdc.flush_all();
    }

    /// Reasserts or clears the SCSI interrupt line (IRQ 8, slave PIC).
    fn refresh_scsi_irq(&mut self) {
        if self.scsi.irq_line() {
            self.pic.set_irq(IRQ_SCSI);
            self.tracer.trace_irq_raise(IRQ_SCSI);
        } else {
            self.pic.clear_irq(IRQ_SCSI);
        }
    }

    /// Rearms the SCSI command task from its next internal deadline.
    fn reschedule_scsi(&mut self) {
        match self.scsi.next_task_cycle() {
            Some(cycle) => self.scheduler.schedule(EventTowns::ScsiTask, cycle),
            None => self.scheduler.cancel(EventTowns::ScsiTask),
        }
        self.update_next_event_cycle();
    }

    /// Dispatches a SCSI I/O read and refreshes its IRQ and scheduled task.
    fn scsi_io_read(&mut self, port: u16) -> u8 {
        let value = self.scsi.io_read(port, self.current_cycle);
        self.refresh_scsi_irq();
        self.reschedule_scsi();
        value
    }

    fn scsi_io_write(&mut self, port: u16, value: u8) {
        self.scsi.io_write(port, value, self.current_cycle);
        self.refresh_scsi_irq();
        self.reschedule_scsi();
    }

    /// Runs the scheduled SCSI task, attempting any DMA data transfer the
    /// controller requests over channel 1. A transfer only moves data once the
    /// host has programmed and unmasked the channel; otherwise it is retried on
    /// the controller's data interval.
    fn service_scsi_task(&mut self) {
        match self.scsi.run_task(self.current_cycle) {
            ScsiDmaRequest::None => {}
            ScsiDmaRequest::DataIn => self.service_scsi_data_in(),
            ScsiDmaRequest::DataOut => self.service_scsi_data_out(),
        }
        self.refresh_scsi_irq();
        self.reschedule_scsi();
    }

    /// Attempts one DATA IN chunk: moves pending target bytes into memory
    /// through DMA channel 1 and signals ~END for the transferred chunk.
    fn service_scsi_data_in(&mut self) {
        if !self.dmac[DMA_MAIN].channel_unmasked(SCSI_DMA_CHANNEL) {
            self.scsi.retry_data_transfer(self.current_cycle);
            return;
        }
        let data = self.scsi.data_in_remaining().to_vec();
        let result = self.dmac[DMA_MAIN].transfer_write_to_memory(SCSI_DMA_CHANNEL, &data);
        let transferred = result.writes.len();
        for (address, byte) in result.writes {
            self.memory.write_byte(address, byte);
        }
        if transferred == 0 {
            self.scsi.retry_data_transfer(self.current_cycle);
        } else {
            self.dmac[DMA_MAIN].set_terminal_count(SCSI_DMA_CHANNEL);
            self.scsi
                .on_data_in_transferred(transferred, self.current_cycle);
        }
    }

    /// Attempts one DATA OUT chunk: collects memory bytes through DMA channel 1
    /// and feeds them to the target, signalling ~END for the collected chunk.
    fn service_scsi_data_out(&mut self) {
        if !self.dmac[DMA_MAIN].channel_unmasked(SCSI_DMA_CHANNEL) {
            self.scsi.retry_data_transfer(self.current_cycle);
            return;
        }
        let remaining = self.scsi.data_out_remaining();
        let result = self.dmac[DMA_MAIN].transfer_read_from_memory(SCSI_DMA_CHANNEL, remaining);
        if result.addresses.is_empty() {
            self.scsi.retry_data_transfer(self.current_cycle);
            return;
        }
        let data: Vec<u8> = result
            .addresses
            .iter()
            .map(|&address| self.memory.read_byte(address))
            .collect();
        self.dmac[DMA_MAIN].set_terminal_count(SCSI_DMA_CHANNEL);
        self.scsi.on_data_out_collected(&data, self.current_cycle);
    }

    /// Attaches a hard disk image at the given SCSI drive index (0-based) and
    /// registers its boot partition in the CMOS drive-assignment table so the
    /// Towns OS mounts a drive letter for it (and can therefore boot from it).
    pub(crate) fn insert_hdd(&mut self, drive: usize, image: HddImage, path: Option<PathBuf>) {
        self.scsi.insert_drive(drive, MountedHdd::new(image, path));
        self.memory.register_scsi_hdd(drive as u8, 0);
    }

    /// Flushes all attached hard disks to their backing files.
    pub(crate) fn flush_hdds(&mut self) {
        self.scsi.flush();
    }

    /// Reasserts or clears the shared sound IRQ 13 line (slave IR5). The OPN2 FM
    /// timers and the RF5C68 PCM interrupt are OR-merged onto it.
    fn refresh_sound_irq(&mut self) {
        if self.fm.irq_asserted() || self.pcm.interrupt_asserted() {
            self.pic.set_irq(IRQ_SOUND);
            self.tracer.trace_irq_raise(IRQ_SOUND);
        } else {
            self.pic.clear_irq(IRQ_SOUND);
        }
    }

    /// Drains the OPN2's pending FM timer requests onto the scheduler and routes
    /// its IRQ edge to the shared sound IRQ.
    fn apply_sound_timers(&mut self) {
        // At most two timer actions; copy them out to release the device borrow.
        let actions: [Option<FmTimerAction>; 2] = {
            let drained = self.fm.drain_timers();
            let mut out = [None, None];
            for (slot, action) in out.iter_mut().zip(drained.iter()) {
                *slot = Some(*action);
            }
            out
        };
        for action in actions.into_iter().flatten() {
            match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => {
                    let kind = if timer_id == 0 {
                        EventTowns::FmTimerA
                    } else {
                        EventTowns::FmTimerB
                    };
                    self.scheduler.schedule(kind, fire_cycle);
                }
                FmTimerAction::Cancel { timer_id } => {
                    let kind = if timer_id == 0 {
                        EventTowns::FmTimerA
                    } else {
                        EventTowns::FmTimerB
                    };
                    self.scheduler.cancel(kind);
                }
            }
        }
        if self.fm.take_irq_change().is_some() {
            self.refresh_sound_irq();
        }
        self.update_next_event_cycle();
    }

    /// Generates and mixes one audio frame from the OPN2 FM chip, the RF5C68 PCM
    /// chip, and CD-DA into `output` (interleaved stereo), returning the number
    /// of samples written.
    ///
    /// The mute latch, the audio-out enable, and the CD electronic volume gate
    /// the mix only: muted chips still advance so their timers, interrupts, and
    /// resamplers stay aligned.
    pub(crate) fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let current_cycle = self.current_cycle;
        let cpu_clock_hz = self.clocks.cpu_clock_hz;

        let output_enabled = self.sound_audio & AUDIO_OUT_ENABLE != 0;
        let fm_volume = if output_enabled && self.sound_mute & MUTE_FM_AUDIBLE != 0 {
            volume
        } else {
            0.0
        };
        let pcm_volume = if output_enabled && self.sound_mute & MUTE_PCM_AUDIBLE != 0 {
            volume
        } else {
            0.0
        };
        let cd_volumes = if output_enabled {
            [
                volume * self.elevol[ELEVOL_CD].channel_ratio(ELEVOL_CD_LEFT),
                volume * self.elevol[ELEVOL_CD].channel_ratio(ELEVOL_CD_RIGHT),
            ]
        } else {
            [0.0, 0.0]
        };

        self.fm
            .generate_samples(current_cycle, cpu_clock_hz, fm_volume, output);
        self.apply_sound_timers();
        self.pcm
            .generate_samples(current_cycle, cpu_clock_hz, pcm_volume, output);
        self.refresh_sound_irq();
        self.cdc.generate_audio_samples(cd_volumes, output);
        // The buzzer is a separate output path from the FM/PCM mixer, so it is
        // not gated by the mute or audio-out latches; it mixes on top of them.
        self.beeper.mix_samples(
            current_cycle,
            cpu_clock_hz,
            TIMER_CLOCK_HZ,
            self.clocks.sample_rate,
            volume,
            output,
        );

        // FM Towns RS-MIDI: bytes the guest transmits on the RS-232C port are
        // captured by the USART and forwarded to whichever module is installed.
        #[cfg(feature = "mt32")]
        if let Some(ref mt32) = self.mt32 {
            mt32.exchange(volume, output, |buf| self.rs232c.flush_midi_into(buf));
        }
        #[cfg(feature = "sc55")]
        if let Some(ref sc55) = self.sc55 {
            sc55.exchange(volume, output, |buf| self.rs232c.flush_midi_into(buf));
        }

        output.len()
    }

    /// Writes the CMOS boot-device type and boot-device bytes the IPL reads.
    pub(crate) fn set_boot_device_cmos(&mut self, device_type: u8, boot_device: u8) {
        self.memory.set_boot_device_cmos(device_type, boot_device);
    }

    /// Reschedules an interrupt-capable timer channel's next edge, or cancels it
    /// when the channel is not producing edges.
    fn reschedule_timer(&mut self, channel: usize) {
        let event = match channel {
            0 => EventTowns::TimerChannel0,
            1 => EventTowns::TimerChannel1,
            _ => return,
        };
        match self
            .timer
            .interrupt_period_cycles(channel, self.clocks.cpu_clock_hz)
        {
            Some(period) => self
                .scheduler
                .schedule(event, self.current_cycle.saturating_add(period.max(1))),
            None => self.scheduler.cancel(event),
        }
        self.update_next_event_cycle();
    }

    /// Recomputes the next scheduled event cycle.
    fn update_next_event_cycle(&mut self) {
        self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
    }

    /// Dispatches all events due at the current cycle and re-arms periodic ones.
    fn process_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        for event in due.iter() {
            match event.kind {
                EventTowns::TimerChannel0 => {
                    self.timer.latch_channel_out(0);
                    self.refresh_timer_irq();
                    self.reschedule_timer(0);
                }
                EventTowns::TimerChannel1 => {
                    self.timer.latch_channel_out(1);
                    self.refresh_timer_irq();
                    // Channel 1 is one-shot; it re-arms only on reprogramming.
                }
                EventTowns::KeyboardReady => {}
                EventTowns::VsyncStart => {
                    self.last_vsync_start_cycle = event.fire_cycle;
                    self.video.enter_vsync();
                    self.memory.set_sync_status(true, false);
                    self.refresh_vsync_irq();
                    // Sprites only transfer when the screen mode presents layer 1
                    // as a 16 bpp 512-byte-per-line page.
                    if self.video.screen_mode_accepts_sprite()
                        && let Some(delay) = self.sprite.on_vsync_start()
                    {
                        self.scheduler.schedule(
                            EventTowns::SpriteFinish,
                            self.current_cycle.saturating_add(delay.max(1)),
                        );
                    }
                    self.render_frame();
                    let duration = self.vsync_duration_cycles();
                    self.scheduler.schedule(
                        EventTowns::VsyncEnd,
                        self.current_cycle.saturating_add(duration),
                    );
                    self.schedule_next_vsync();
                }
                EventTowns::VsyncEnd => {
                    self.video.leave_vsync();
                    self.memory.set_sync_status(false, false);
                }
                EventTowns::CdTask => {
                    self.service_cdrom_task();
                }
                EventTowns::FmTimerA => {
                    self.fm.timer_expired(0, event.fire_cycle);
                    self.apply_sound_timers();
                }
                EventTowns::FmTimerB => {
                    self.fm.timer_expired(1, event.fire_cycle);
                    self.apply_sound_timers();
                }
                EventTowns::SpriteFinish => {
                    if let Some(params) = self.sprite.on_finish() {
                        self.memory.render_sprites(&params);
                    }
                }
                EventTowns::FdcTask => {
                    self.service_fdc_task();
                }
                EventTowns::ScsiTask => {
                    self.service_scsi_task();
                }
            }
        }
        self.update_next_event_cycle();
    }

    /// The vertical-sync pulse duration in CPU cycles.
    fn vsync_duration_cycles(&self) -> u64 {
        (u64::from(self.clocks.cpu_clock_hz) * VSYNC_DURATION_MICROS / MICROS_PER_SECOND).max(1)
    }
}

impl<T: Tracing> Bus for TownsBus<T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        if (RF5C68_WAVE_WINDOW_BASE..RF5C68_WAVE_WINDOW_END).contains(&address) {
            return self
                .pcm
                .read_wave_ram((address - RF5C68_WAVE_WINDOW_BASE) as u16);
        }
        if address == TOWNSMEMIO_BUZZER_CONTROL && self.memory.fmr_window_mapped() {
            self.buzzer_memio = true;
            self.refresh_beeper_gate();
        }
        self.memory.read_byte(address)
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        if (RF5C68_WAVE_WINDOW_BASE..RF5C68_WAVE_WINDOW_END).contains(&address) {
            self.pcm
                .write_wave_ram((address - RF5C68_WAVE_WINDOW_BASE) as u16, value);
            return;
        }
        if address == TOWNSMEMIO_BUZZER_CONTROL && self.memory.fmr_window_mapped() {
            self.buzzer_memio = false;
            self.refresh_beeper_gate();
        }
        self.memory.write_byte(address, value);
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        let value = self.io_read(port);
        self.tracer.trace_io_read(port, value);
        value
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        self.tracer.trace_io_write(port, value);
        self.io_write(port, value);
    }

    fn io_write_word(&mut self, port: u16, value: u16) {
        // The high-res "image out" register file uses 16/32-bit accesses: the
        // index latch takes a full 16-bit word, and a 32-bit register write
        // arrives as a low word to 0x0474 then a high word to 0x0476 (the latter
        // completing the access and advancing the palette index). Everything
        // else keeps the default two-byte decomposition.
        match port {
            0x0472 => {
                self.tracer.trace_io_write(port, value as u8);
                self.video.write_high_res_addr_word(value);
            }
            0x0474 => {
                self.tracer.trace_io_write(port, value as u8);
                self.video.write_high_res_data_low_word(value);
            }
            0x0476 => {
                self.tracer.trace_io_write(port, value as u8);
                self.video.write_high_res_data_high_word(value);
            }
            _ => {
                self.io_write_byte(port, value as u8);
                self.io_write_byte(port.wrapping_add(1), (value >> 8) as u8);
            }
        }
    }

    fn has_irq(&self) -> bool {
        self.pic.has_pending_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        self.pic.acknowledge()
    }

    fn reset_pending(&self) -> bool {
        self.soft_reset_pending || self.power_off_requested
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
}
