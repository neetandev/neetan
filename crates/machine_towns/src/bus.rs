//! FM Towns system bus.
//!
//! Owns the memory map and the core chipset (dual 8259 PICs, the interval
//! timer, the two uPD71071 DMA controllers, the MSM58321 RTC, and the serial
//! keyboard), and dispatches the CPU's memory and I/O accesses. The FM Towns
//! decodes the full 16-bit I/O port address, so the dispatch matches on the
//! whole port value.

mod io_read;
mod io_write;

use std::path::PathBuf;

use common::{
    BeeperKind, Bus, HostDateTimeProvider, NoTrace, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField,
    TraceInterruptAction, TraceInterruptKind, TracePresentation, TraceSink, TraceValue, trace_id,
};
use device::{
    beeper::Beeper,
    cdrom_towns::TownsCdController,
    disk::{HddImage, MountedHdd},
    electronic_volume_towns::ElectronicVolume,
    floppy::{FloppyImage, MountedFloppy},
    gameport_towns::TownsGamePort,
    i8251_serial::I8251Serial,
    i8259a_pic::I8259aPic,
    keyboard_towns::TownsKeyboard,
    msm58321_rtc::Msm58321Rtc,
    opn_fm::{FmTimerAction, OpnFm, Ymf276},
    rf5c68::Rf5c68,
    scsi::{ScsiDmaRequest, TownsScsiController},
    sprite_towns::TownsSprite,
    timer_towns::{TIMER_CLOCK_HZ, TownsTimer},
    upd71071_dma::Upd71071Dma,
    video_towns::TownsVideo,
    wd17xx_fdc::{WD17XX_PLATFORM_FM_TOWNS, Wd17xxFdc},
};
use software_renderer::{RenderInputsTowns, TownsRenderer};

use crate::{
    config::{ClockConfig, TownsModel},
    memory::TownsMemory,
    rom::LoadedRoms,
    scheduler::{EventTowns, TownsScheduler},
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

/// Baseline wait-state cycles charged on every video-memory access on top of the
/// programmed VRAM wait latch, modeling VRAM being inherently slower than RAM.
/// Experimental tuning knob.
const VRAM_BASELINE_WAIT_CYCLES: i64 = 2;

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

#[cfg(feature = "mt32")]
type TownsMt32State = device::mt32::MuntActorState;
#[cfg(not(feature = "mt32"))]
save_state::runtime_state! {
/// Empty MT-32 state used when MT-32 support is not compiled in.
#[derive(Clone)]
struct TownsMt32State {}
}

#[cfg(feature = "sc55")]
type TownsSc55State = device::sc55::Sc55ActorState;
#[cfg(not(feature = "sc55"))]
save_state::runtime_state! {
/// Empty SC-55 state used when SC-55 support is not compiled in.
#[derive(Clone)]
struct TownsSc55State {}
}

save_state::runtime_state! {
/// Complete authoritative FM Towns bus state.
#[derive(Clone)]
pub(crate) struct TownsBusState {
    memory: crate::memory::TownsMemoryState,
    current_cycle: u64,
    scheduler: crate::scheduler::TownsSchedulerState,
    pic: device::i8259a_pic::I8259aPicState,
    timer: device::timer_towns::TownsTimerRuntimeState,
    dmac: [device::upd71071_dma::Upd71071State; 2],
    rtc: device::msm58321_rtc::Msm58321RuntimeState,
    keyboard: device::keyboard_towns::TownsKeyboardState,
    cdc: device::cdrom_towns::TownsCdControllerState,
    fdc: device::wd17xx_fdc::Wd17xxFdcState,
    scsi: device::scsi::TownsScsiControllerState,
    beeper: device::beeper::BeeperState,
    buzzer_memio: bool,
    fm: device::opn_fm::Ymf276RuntimeState,
    pcm: device::rf5c68::Rf5c68RuntimeState,
    sound_mute: u8,
    sound_audio: u8,
    elevol: [device::electronic_volume_towns::ElectronicVolumeState; 2],
    main_ram_wait: u8,
    vram_wait: u8,
    pending_wait_cycles: i64,
    gameport: device::gameport_towns::TownsGamePortState,
    video: device::video_towns::TownsVideoRuntimeState,
    sprite: device::sprite_towns::TownsSpriteRuntimeState,
    renderer: software_renderer::towns::TownsRendererRuntimeState,
    display_width: u32,
    display_height: u32,
    presented_frames: u64,
    model: u8,
    machine_id: (u8, u8),
    last_vsync_start_cycle: u64,
    nmi_mask: u8,
    rs232c: device::i8251_serial::I8251SerialState,
    rs232c_int_enable: u8,
    reset_reason: u8,
    soft_reset_pending: bool,
    power_off_requested: bool,
    serial_rom_bit_count: u8,
    last_serial_rom_command: u8,
    memcard_bank: u8,
    memcard_reg: bool,
    mt32: Option<TownsMt32State>,
    sc55: Option<TownsSc55State>,
}}

/// Default host time source (a fixed timestamp) until the app installs one.
/// The FM Towns system bus.
pub struct TownsBus<T: TraceSink = NoTrace> {
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
    pub(crate) fdc: Wd17xxFdc<WD17XX_PLATFORM_FM_TOWNS>,
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
    /// Main-RAM wait latch (I/O 0x05E0 first-generation alias / 0x05E2). Charged
    /// as wait-state cycles on every non-video memory access (RAM, ROM, CMOS).
    pub(crate) main_ram_wait: u8,
    /// VRAM wait latch (I/O 0x05E6). Charged as wait-state cycles on every video
    /// memory access (native VRAM, sprite RAM, and the mapped FMR VRAM window).
    pub(crate) vram_wait: u8,
    /// Memory wait-state cycles accumulated since the last drain. The CPU pulls
    /// these through [`Bus::drain_wait_cycles`] after each instruction.
    pub(crate) pending_wait_cycles: i64,
    pub(crate) gameport: TownsGamePort,
    pub(crate) video: TownsVideo,
    pub(crate) sprite: TownsSprite,
    pub(crate) renderer: TownsRenderer,
    /// Valid display width from the last composed frame.
    pub(crate) display_width: u32,
    /// Valid display height from the last composed frame.
    pub(crate) display_height: u32,
    /// Number assigned to the next published frame.
    pub(crate) presented_frames: u64,
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
    pub(crate) host_date_time_provider: HostDateTimeProvider,
    /// Roland MT-32 sound module, fed by RS-MIDI bytes (optional, requires munt).
    #[cfg(feature = "mt32")]
    mt32: Option<device::mt32::Mt32>,
    /// Roland SC-55 sound module, fed by RS-MIDI bytes (optional, requires Nuked-SC55).
    #[cfg(feature = "sc55")]
    sc55: Option<device::sc55::Sc55>,
    pub(crate) tracer: T,
}

impl<T: TraceSink> TownsBus<T> {
    /// Builds a traced bus for a model from its validated ROM set.
    pub fn new_with_trace_sink(
        model: TownsModel,
        cpu_mode: common::CpuMode,
        roms: LoadedRoms,
        sample_rate: u32,
        tracer: T,
    ) -> Self {
        let clocks = ClockConfig {
            cpu_clock_hz: model.cpu_clock_hz(cpu_mode),
            sample_rate,
        };
        Self::from_parts(TownsMemory::new(model, roms), clocks, model, tracer)
    }

    /// Builds the bus over a prepared memory map and clock configuration.
    pub(crate) fn from_parts(
        memory: TownsMemory,
        clocks: ClockConfig,
        model: TownsModel,
        tracer: T,
    ) -> Self {
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
            fdc: Wd17xxFdc::new(clocks.cpu_clock_hz),
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
            pending_wait_cycles: 0,
            gameport: TownsGamePort::new(clocks.cpu_clock_hz),
            video: TownsVideo::new(model.high_res_available()),
            sprite: TownsSprite::new(clocks.cpu_clock_hz),
            renderer: TownsRenderer::new(),
            display_width: 640,
            display_height: 480,
            presented_frames: 0,
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
            host_date_time_provider: common::default_host_date_time,
            #[cfg(feature = "mt32")]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: None,
            tracer,
        };
        bus.schedule_next_vsync();
        bus
    }

    /// Returns stable identities for installed ROM resources.
    pub(crate) fn save_state_resources(
        &self,
    ) -> Result<save_state::ResourceManifest, save_state::StateValidationError> {
        save_state::ResourceManifest::new(self.memory.resource_bindings()?)
    }

    /// Returns stable identities for every mounted medium.
    pub(crate) fn save_state_media(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = Vec::new();
        bindings.extend_from_slice(self.cdc.media_manifest()?.bindings());
        bindings.extend_from_slice(self.fdc.media_manifest()?.bindings());
        bindings.extend_from_slice(self.scsi.media_manifest()?.bindings());
        save_state::MediaManifest::new(bindings)
    }

    /// Captures the complete FM Towns bus at a machine safe point.
    pub(crate) fn capture_runtime_state(
        &mut self,
    ) -> Result<TownsBusState, save_state::SaveStateError> {
        Ok(TownsBusState {
            memory: self.memory.capture_state(),
            current_cycle: self.current_cycle,
            scheduler: self.scheduler.capture_state(),
            pic: self.pic.capture_state(),
            timer: self.timer.capture_state(),
            dmac: [self.dmac[0].state.clone(), self.dmac[1].state.clone()],
            rtc: self.rtc.capture_state(),
            keyboard: self.keyboard.capture_state(),
            cdc: self.cdc.capture_state()?,
            fdc: self.fdc.capture_state()?,
            scsi: self.scsi.capture_state()?,
            beeper: self.beeper.capture_state(),
            buzzer_memio: self.buzzer_memio,
            fm: self.fm.capture_state(),
            pcm: self.pcm.capture_state(),
            sound_mute: self.sound_mute,
            sound_audio: self.sound_audio,
            elevol: [
                self.elevol[0].capture_state(),
                self.elevol[1].capture_state(),
            ],
            main_ram_wait: self.main_ram_wait,
            vram_wait: self.vram_wait,
            pending_wait_cycles: self.pending_wait_cycles,
            gameport: self.gameport.capture_state(),
            video: self.video.capture_state(),
            sprite: self.sprite.capture_state(),
            renderer: self.renderer.capture_state(),
            display_width: self.display_width,
            display_height: self.display_height,
            presented_frames: self.presented_frames,
            model: match self.model {
                TownsModel::FmTowns => 0,
                TownsModel::FmTownsIICx => 1,
                TownsModel::FmTownsIIMx => 2,
            },
            machine_id: self.machine_id,
            last_vsync_start_cycle: self.last_vsync_start_cycle,
            nmi_mask: self.nmi_mask,
            rs232c: self.rs232c.state.clone(),
            rs232c_int_enable: self.rs232c_int_enable,
            reset_reason: self.reset_reason,
            soft_reset_pending: self.soft_reset_pending,
            power_off_requested: self.power_off_requested,
            serial_rom_bit_count: self.serial_rom_bit_count,
            last_serial_rom_command: self.last_serial_rom_command,
            memcard_bank: self.memcard_bank,
            memcard_reg: self.memcard_reg,
            #[cfg(feature = "mt32")]
            mt32: self
                .mt32
                .as_mut()
                .map(device::mt32::Mt32::capture_state)
                .transpose()
                .map_err(|error| save_state::SaveStateError::WorkerFailure(error.to_string()))?,
            #[cfg(not(feature = "mt32"))]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: self
                .sc55
                .as_mut()
                .map(device::sc55::Sc55::capture_state)
                .transpose()
                .map_err(|error| save_state::SaveStateError::WorkerFailure(error.to_string()))?,
            #[cfg(not(feature = "sc55"))]
            sc55: None,
        })
    }

    /// Restores the complete FM Towns bus while retaining host resources.
    pub(crate) fn restore_runtime_state(
        &mut self,
        state: TownsBusState,
    ) -> Result<(), save_state::SaveStateError> {
        let model = match state.model {
            0 => TownsModel::FmTowns,
            1 => TownsModel::FmTownsIICx,
            2 => TownsModel::FmTownsIIMx,
            _ => {
                return Err(
                    save_state::StateValidationError::new("FM Towns model is invalid").into(),
                );
            }
        };
        #[cfg(feature = "mt32")]
        let mt32_configuration_differs = state.mt32.is_some() != self.mt32.is_some();
        #[cfg(not(feature = "mt32"))]
        let mt32_configuration_differs = false;
        #[cfg(feature = "sc55")]
        let sc55_configuration_differs = state.sc55.is_some() != self.sc55.is_some();
        #[cfg(not(feature = "sc55"))]
        let sc55_configuration_differs = false;
        if model != self.model
            || state.machine_id != self.machine_id
            || mt32_configuration_differs
            || sc55_configuration_differs
        {
            return Err(save_state::StateValidationError::new(
                "FM Towns machine configuration differs",
            )
            .into());
        }
        for controller in &state.dmac {
            if controller.selected_channel >= controller.channels.len() {
                return Err(save_state::StateValidationError::new(
                    "FM Towns DMA channel selection is invalid",
                )
                .into());
            }
        }

        #[cfg(feature = "mt32")]
        let mut mt32_prepared = false;
        #[cfg(feature = "mt32")]
        if let (Some(module), Some(saved)) = (&mut self.mt32, state.mt32.clone()) {
            module
                .prepare_restore(saved)
                .map_err(|error| save_state::SaveStateError::WorkerFailure(error.to_string()))?;
            mt32_prepared = true;
        }
        #[cfg(feature = "sc55")]
        let mut sc55_prepared = false;
        #[cfg(feature = "sc55")]
        if let (Some(module), Some(saved)) = (&mut self.sc55, state.sc55.clone()) {
            if let Err(error) = module.prepare_restore(saved) {
                #[cfg(feature = "mt32")]
                if mt32_prepared && let Some(module) = &mut self.mt32 {
                    let _ = module.abort_restore();
                }
                return Err(save_state::SaveStateError::WorkerFailure(error.to_string()));
            }
            sc55_prepared = true;
        }

        let restore_result = (|| -> Result<(), save_state::SaveStateError> {
            self.memory.restore_state(state.memory)?;
            self.scheduler.restore_state(state.scheduler)?;
            self.pic.restore_state(state.pic)?;
            self.timer.restore_state(state.timer)?;
            self.rtc.restore_state(state.rtc)?;
            self.keyboard.restore_state(state.keyboard)?;
            self.cdc.restore_state(state.cdc)?;
            self.fdc.restore_state(state.fdc)?;
            self.scsi.restore_state(state.scsi)?;
            self.beeper.restore_state(state.beeper)?;
            self.fm.restore_state(state.fm)?;
            self.pcm.restore_state(state.pcm)?;
            self.elevol[0].restore_state(state.elevol[0].clone())?;
            self.elevol[1].restore_state(state.elevol[1].clone())?;
            self.gameport.restore_state(state.gameport)?;
            self.video.restore_state(state.video)?;
            self.sprite.restore_state(state.sprite)?;
            self.renderer.restore_state(state.renderer)?;

            let [main_dma, extended_dma] = state.dmac;
            self.dmac[0].state = main_dma;
            self.dmac[1].state = extended_dma;
            self.current_cycle = state.current_cycle;
            self.buzzer_memio = state.buzzer_memio;
            self.sound_mute = state.sound_mute;
            self.sound_audio = state.sound_audio;
            self.main_ram_wait = state.main_ram_wait;
            self.vram_wait = state.vram_wait;
            self.pending_wait_cycles = state.pending_wait_cycles;
            self.display_width = state.display_width;
            self.display_height = state.display_height;
            self.presented_frames = state.presented_frames;
            self.last_vsync_start_cycle = state.last_vsync_start_cycle;
            self.nmi_mask = state.nmi_mask;
            self.rs232c.state = state.rs232c;
            self.rs232c_int_enable = state.rs232c_int_enable;
            self.reset_reason = state.reset_reason;
            self.soft_reset_pending = state.soft_reset_pending;
            self.power_off_requested = state.power_off_requested;
            self.serial_rom_bit_count = state.serial_rom_bit_count;
            self.last_serial_rom_command = state.last_serial_rom_command;
            self.memcard_bank = state.memcard_bank;
            self.memcard_reg = state.memcard_reg;
            self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
            Ok(())
        })();

        #[cfg(any(feature = "mt32", feature = "sc55"))]
        if let Err(error) = restore_result {
            #[cfg(feature = "mt32")]
            if mt32_prepared && let Some(module) = &mut self.mt32 {
                let _ = module.abort_restore();
            }
            #[cfg(feature = "sc55")]
            if sc55_prepared && let Some(module) = &mut self.sc55 {
                let _ = module.abort_restore();
            }
            return Err(error);
        }
        #[cfg(not(any(feature = "mt32", feature = "sc55")))]
        restore_result?;

        #[cfg(feature = "mt32")]
        if mt32_prepared
            && let Some(module) = &mut self.mt32
            && let Err(error) = module.commit_restore()
        {
            #[cfg(feature = "sc55")]
            if sc55_prepared && let Some(module) = &mut self.sc55 {
                let _ = module.abort_restore();
            }
            return Err(save_state::SaveStateError::WorkerFailure(error.to_string()));
        }
        #[cfg(feature = "sc55")]
        if sc55_prepared
            && let Some(module) = &mut self.sc55
            && let Err(error) = module.commit_restore()
        {
            return Err(save_state::SaveStateError::WorkerFailure(error.to_string()));
        }
        Ok(())
    }
}

impl TownsBus<NoTrace> {
    /// Builds an untraced bus for a model from its validated ROM set.
    pub fn new(
        model: TownsModel,
        cpu_mode: common::CpuMode,
        roms: LoadedRoms,
        sample_rate: u32,
    ) -> Self {
        Self::new_with_trace_sink(model, cpu_mode, roms, sample_rate, NoTrace)
    }
}

impl<T: TraceSink> TownsBus<T> {
    /// Overrides the host local-time source (BCD) used by the RTC.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
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

    /// Copies RS-232C MIDI into `target` and returns the number of bytes written.
    pub fn flush_midi_into(&mut self, target: &mut [u8]) -> usize {
        self.rs232c.flush_midi_into(target)
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

    /// Sets a PIC IRQ line and traces state transitions.
    fn set_pic_irq(&mut self, irq: u8, asserted: bool) {
        let changed = if asserted {
            self.pic.set_irq(irq)
        } else {
            self.pic.clear_irq(irq)
        };
        if T::ENABLED && changed {
            let action = if asserted {
                TraceInterruptAction::Assert
            } else {
                TraceInterruptAction::Clear
            };
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::maskable_interrupt(
                    trace_id::controller::TOWNS_PIC,
                    u16::from(irq),
                    action,
                    None,
                ),
            );
        }
    }

    /// Reasserts or clears the timer IRQ 0 line into the master PIC.
    fn refresh_timer_irq(&mut self) {
        self.set_pic_irq(0, self.timer.irq_active());
    }

    /// Reasserts or clears the VSYNC IRQ 11 line (slave IR3) into the PIC.
    fn refresh_vsync_irq(&mut self) {
        self.set_pic_irq(IRQ_VSYNC, self.video.vsync_irq_pending());
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

    /// Reasserts or clears the RS-232C IRQ 2 line into the master PIC. The
    /// receiver-ready source is gated by its interrupt-enable bit (0x0A08).
    fn refresh_rs232c_irq(&mut self) {
        let rx_int = self.rs232c_int_enable & RS232C_INT_ENABLE_RXRDY != 0
            && self.rs232c.read_status() & 0x02 != 0;
        self.set_pic_irq(IRQ_RS232C, rx_int);
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
        self.set_pic_irq(1, self.keyboard.irq_line());
    }

    /// Reasserts or clears the CD-ROM IRQ 9 line (slave IR1) into the PIC.
    fn refresh_cdrom_irq(&mut self) {
        let (status_irq, data_end_irq) = self.cdc.interrupt_flags();
        if T::ENABLED
            && self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::TOWNS_CDROM,
                action: trace_id::action::INTERRUPT,
            })
        {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::Device(TraceDeviceEvent {
                    device: trace_id::device::TOWNS_CDROM,
                    action: trace_id::action::INTERRUPT,
                    fields: &[
                        TraceField {
                            name: trace_id::field::STATUS,
                            value: TraceValue::Bool(status_irq),
                        },
                        TraceField {
                            name: trace_id::field::DATA_END,
                            value: TraceValue::Bool(data_end_irq),
                        },
                    ],
                }),
            );
        }
        self.set_pic_irq(IRQ_CDROM, self.cdc.irq_line());
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
        if port == 0x04C2
            && T::ENABLED
            && self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::TOWNS_CDROM,
                action: trace_id::action::STATUS,
            })
        {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::Device(TraceDeviceEvent {
                    device: trace_id::device::TOWNS_CDROM,
                    action: trace_id::action::STATUS,
                    fields: &[TraceField {
                        name: trace_id::field::BYTES,
                        value: TraceValue::Bytes(&[value]),
                    }],
                }),
            );
        }
        self.refresh_cdrom_irq();
        self.reschedule_cdrom();
        value
    }

    fn cdrom_io_write(&mut self, port: u16, value: u8) {
        let trace_command = port == 0x04C2
            && T::ENABLED
            && self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::TOWNS_CDROM,
                action: trace_id::action::COMMAND,
            });
        let mut trace_parameters = [0; 8];
        let trace_parameter_count = if trace_command {
            let parameters = self.cdc.params();
            trace_parameters[..parameters.len()].copy_from_slice(parameters);
            parameters.len()
        } else {
            0
        };
        let dma_ready = self.cdrom_dma_ready();
        self.cdc
            .io_write(port, value, self.current_cycle, dma_ready);
        if trace_command {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::Device(TraceDeviceEvent {
                    device: trace_id::device::TOWNS_CDROM,
                    action: trace_id::action::COMMAND,
                    fields: &[
                        TraceField {
                            name: trace_id::field::OPCODE,
                            value: TraceValue::Unsigned(u64::from(value)),
                        },
                        TraceField {
                            name: trace_id::field::PARAMETERS,
                            value: TraceValue::Bytes(&trace_parameters[..trace_parameter_count]),
                        },
                    ],
                }),
            );
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
        self.set_pic_irq(IRQ_FDC, self.fdc.irq_line());
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
        self.set_pic_irq(IRQ_SCSI, self.scsi.irq_line());
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
        self.set_pic_irq(
            IRQ_SOUND,
            self.fm.irq_asserted() || self.pcm.interrupt_asserted(),
        );
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
        if let Some(ref mut mt32) = self.mt32 {
            mt32.exchange(volume, output, |buffer| self.rs232c.flush_midi_into(buffer));
        }
        #[cfg(feature = "sc55")]
        if let Some(ref mut sc55) = self.sc55 {
            sc55.exchange(volume, output, |buffer| self.rs232c.flush_midi_into(buffer));
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
                    self.trace_presentation();
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

    /// Charges one memory access at `address` its programmed wait-state cycles:
    /// the VRAM wait latch for video memory, the main-RAM wait latch otherwise.
    /// Called once per access regardless of width, matching a single bus cycle
    /// on the 32-bit data bus; the 386SX 16-bit split penalty is charged by the
    /// CPU core.
    fn charge_memory_wait(&mut self, address: u32) {
        if self.memory.is_video_memory(address) {
            self.pending_wait_cycles += VRAM_BASELINE_WAIT_CYCLES + i64::from(self.vram_wait);
        } else {
            self.pending_wait_cycles += i64::from(self.main_ram_wait);
        }
    }

    /// Reads a byte through the memory-mapped windows (RF5C68 wave RAM, FMR
    /// buzzer control) and the physical memory map, without charging waits.
    fn read_byte_data(&mut self, address: u32) -> u8 {
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

    /// Writes a byte through the memory-mapped windows and the physical memory
    /// map, without charging waits.
    fn write_byte_data(&mut self, address: u32, value: u8) {
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

    fn read_byte_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u8 {
        self.charge_memory_wait(address);
        let value = self.read_byte_data(address);
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
                    u64::from(address),
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    /// Reads one word with a single wait charge and the requested trace kind.
    fn read_word_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u16 {
        self.charge_memory_wait(address);
        let value = u16::from(self.read_byte_data(address))
            | (u16::from(self.read_byte_data(address.wrapping_add(1))) << 8);
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
                    u64::from(address),
                    TraceAccessWidth::Word,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }

    /// Reads one doubleword with a single wait charge and the requested trace kind.
    fn read_dword_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u32 {
        self.charge_memory_wait(address);
        let value = u32::from(self.read_byte_data(address))
            | (u32::from(self.read_byte_data(address.wrapping_add(1))) << 8)
            | (u32::from(self.read_byte_data(address.wrapping_add(2))) << 16)
            | (u32::from(self.read_byte_data(address.wrapping_add(3))) << 24);
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
                    u64::from(address),
                    TraceAccessWidth::Dword,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
        value
    }
}

impl<T: TraceSink> Bus for TownsBus<T> {
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
        self.charge_memory_wait(address);
        self.write_byte_data(address, value);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
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

    fn read_word(&mut self, address: u32) -> u16 {
        self.read_word_for_cpu::<false>(address)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        self.charge_memory_wait(address);
        self.write_byte_data(address, value as u8);
        self.write_byte_data(address.wrapping_add(1), (value >> 8) as u8);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Word,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn read_dword(&mut self, address: u32) -> u32 {
        self.read_dword_for_cpu::<false>(address)
    }

    fn write_dword(&mut self, address: u32, value: u32) {
        self.charge_memory_wait(address);
        self.write_byte_data(address, value as u8);
        self.write_byte_data(address.wrapping_add(1), (value >> 8) as u8);
        self.write_byte_data(address.wrapping_add(2), (value >> 16) as u8);
        self.write_byte_data(address.wrapping_add(3), (value >> 24) as u8);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::MAIN_MEMORY,
                    TraceAccessKind::Write,
                    u64::from(address),
                    TraceAccessWidth::Dword,
                    Some(u64::from(value)),
                    true,
                ),
            );
        }
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.pending_wait_cycles)
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.tracer.yield_requested()
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
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

    fn io_write_word(&mut self, port: u16, value: u16) {
        // The high-res "image out" register file uses 16/32-bit accesses: the
        // index latch takes a full 16-bit word, and a 32-bit register write
        // arrives as a low word to 0x0474 then a high word to 0x0476 (the latter
        // completing the access and advancing the palette index). Everything
        // else keeps the default two-byte decomposition.
        match port {
            0x0472 => {
                self.video.write_high_res_addr_word(value);
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
            }
            0x0474 => {
                self.video.write_high_res_data_low_word(value);
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
            }
            0x0476 => {
                self.video.write_high_res_data_high_word(value);
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
        let acknowledge = self.pic.acknowledge_with_line();
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::main_cpu(
                    self.current_cycle,
                    Some(u64::from(self.clocks.cpu_clock_hz)),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::TOWNS_PIC,
                    TraceInterruptKind::Maskable,
                    acknowledge.line.map(u16::from),
                    TraceInterruptAction::Acknowledge,
                    Some(u32::from(acknowledge.vector)),
                ),
            );
        }
        acknowledge.vector
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

#[cfg(test)]
mod tests {
    use common::{
        Bus, CpuMode, TraceAccess, TraceAccessKind, TraceAccessWidth, TraceContext, TraceEvent,
        TraceInterrupt, TraceInterruptAction, TraceSink,
    };

    use super::TownsBus;
    use crate::{config::TownsModel, rom::LoadedRoms};

    #[derive(Default)]
    struct AccessTrace {
        kinds: Vec<TraceAccessKind>,
        accesses: Vec<TraceAccess>,
        interrupts: Vec<TraceInterrupt>,
        command_parameters: Vec<Vec<u8>>,
    }

    impl TraceSink for AccessTrace {
        fn trace(&mut self, _context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Access(access) => {
                    self.kinds.push(access.kind);
                    self.accesses.push(access);
                }
                TraceEvent::Interrupt(interrupt) => self.interrupts.push(interrupt),
                TraceEvent::Device(device)
                    if device.device == common::trace_id::device::TOWNS_CDROM
                        && device.action == common::trace_id::action::COMMAND =>
                {
                    let parameters = device
                        .fields
                        .iter()
                        .find(|field| field.name == common::trace_id::field::PARAMETERS)
                        .and_then(|field| match field.value {
                            common::TraceValue::Bytes(parameters) => Some(parameters.to_vec()),
                            _ => None,
                        })
                        .expect("command parameters field");
                    self.command_parameters.push(parameters);
                }
                _ => {}
            }
        }
    }

    fn traced_bus() -> TownsBus<AccessTrace> {
        let roms = LoadedRoms {
            dos: vec![0; 0x8_0000],
            font: vec![0; 0x4_0000],
            system: vec![0; 0x4_0000],
            f20: vec![0; 0x8_0000],
            dictionary: vec![0; 0x8_0000],
            serial: vec![0; 0x20],
        };
        TownsBus::new_with_trace_sink(
            TownsModel::FmTownsIIMx,
            CpuMode::High,
            roms,
            48_000,
            AccessTrace::default(),
        )
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
    fn wide_opcode_fetches_keep_width_and_single_wait_charge() {
        let mut bus = traced_bus();
        bus.main_ram_wait = 3;

        Bus::fetch_opcode_word(&mut bus, 0);
        assert_eq!(Bus::drain_wait_cycles(&mut bus), 3);
        Bus::fetch_opcode_dword(&mut bus, 4);
        assert_eq!(Bus::drain_wait_cycles(&mut bus), 3);

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
    fn cdrom_command_trace_preserves_submitted_parameters() {
        let mut bus = traced_bus();
        let parameters = [0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00];

        for parameter in parameters {
            Bus::io_write_byte(&mut bus, 0x04C4, parameter);
        }
        Bus::io_write_byte(&mut bus, 0x04C2, 0x04);

        assert_eq!(bus.tracer().command_parameters, [parameters]);
    }

    #[test]
    fn duplicate_irq_updates_trace_only_transitions() {
        let mut bus = traced_bus();

        bus.set_pic_irq(8, true);
        bus.set_pic_irq(8, true);
        bus.set_pic_irq(8, false);
        bus.set_pic_irq(8, false);

        assert_eq!(bus.tracer().interrupts.len(), 2);
        assert_eq!(bus.tracer().interrupts[0].line, Some(8));
        assert_eq!(
            bus.tracer().interrupts[0].action,
            TraceInterruptAction::Assert
        );
        assert_eq!(bus.tracer().interrupts[1].line, Some(8));
        assert_eq!(
            bus.tracer().interrupts[1].action,
            TraceInterruptAction::Clear
        );
    }

    #[test]
    fn interrupt_acknowledgement_is_traced() {
        let mut bus = traced_bus();
        bus.pic.write_port0(0, 0x11);
        bus.pic.write_port2(0, 0x08);
        bus.pic.write_port2(0, 0x80);
        bus.pic.write_port2(0, 0x01);
        bus.pic.write_port2(0, 0x00);
        bus.pic.write_port0(1, 0x11);
        bus.pic.write_port2(1, 0x10);
        bus.pic.write_port2(1, 0x07);
        bus.pic.write_port2(1, 0x01);
        bus.pic.write_port2(1, 0x00);
        bus.pic.set_irq(8);

        let vector = Bus::acknowledge_irq(&mut bus);

        assert_eq!(bus.tracer().interrupts.len(), 1);
        assert_eq!(bus.tracer().interrupts[0].line, Some(8));
        assert_eq!(bus.tracer().interrupts[0].vector, Some(u32::from(vector)));
    }
}
