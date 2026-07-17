//! MSX system bus.
//!
//! CPU and scheduler timestamps use Z80 T-states. Bus accesses within an
//! instruction observe its boundary timestamp, and waits are drained after the
//! instruction completes.

mod controller;
mod fdc;
mod kanji;
mod keyboard;
mod ppi;
mod psg;
mod s1985;

use std::path::Path;

use common::{
    HostDateTime, HostDateTimeProvider, NoTrace, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceContext, TraceDeviceEvent, TraceEvent, TraceEventKey, TraceField,
    TraceInterruptAction, TraceInterruptKind, TracePresentation, TraceSink, TraceValue, trace_id,
};
use device::{
    msx_audio::FsCa1,
    opn_fm::{FmTimerAction, OpnFm, YM2413_INSTRUMENT_DATA_SIZE, Ym2413},
    rp5c01::Rp5c01,
    video_msx::{MsxVdp, MsxVdpEffects, MsxVdpRenderState},
    wd17xx_fdc::{WD17XX_PLATFORM_MSX, Wd17xxFdc},
};
use software_renderer::{MsxRenderer, RenderInputsMsx};

pub use self::controller::{MsxControllerDevice, MsxJoystickState};
use self::{
    kanji::MsxKanjiRom,
    keyboard::MsxKeyboard,
    ppi::MsxPpi,
    psg::MsxPsg,
    s1985::{S1985, S1985_DEVICE_ID},
};
use crate::{
    CartridgeError, CartridgeLoadInfo, FirmwareInstallError, LoadedFirmware, MsxDiskController,
    MsxModel,
    clock::{MsxClock, NTSC_TOTAL_SCANLINES},
    memory::{MsxMemory, OPEN_BUS, SecondarySlotChange},
    scheduler::{EventMsx, MsxScheduler},
};

/// Size limit of a synthetic Z80 program.
const SYNTHETIC_ADDRESS_SPACE_SIZE: usize = 0x1_0000;
/// Wait cycles charged for each M1 opcode fetch.
const M1_WAIT_CYCLES: i64 = 1;
/// First MSX2 switched-I/O port.
const SWITCHED_IO_PORT_START: u8 = 0x40;
/// First switched-I/O device register after the selection port.
const SWITCHED_IO_DEVICE_PORT_START: u8 = 0x41;
/// Last MSX2 switched-I/O port.
const SWITCHED_IO_PORT_END: u8 = 0x4F;
/// YM2413 address port.
const OPLL_ADDRESS_PORT: u8 = 0x7C;
/// YM2413 data port.
const OPLL_DATA_PORT: u8 = 0x7D;
/// First Sony printer port.
const PRINTER_PORT_START: u8 = 0x90;
/// Last Sony printer port.
const PRINTER_PORT_END: u8 = 0x97;
/// VRAM data port.
const VDP_DATA_PORT: u8 = 0x98;
/// VDP control and status port.
const VDP_CONTROL_PORT: u8 = 0x99;
/// V9938 palette port.
const VDP_PALETTE_PORT: u8 = 0x9A;
/// V9938 indirect-register port.
const VDP_INDIRECT_PORT: u8 = 0x9B;
/// First YM2149 I/O port.
const PSG_PORT_START: u8 = 0xA0;
/// Last YM2149 I/O port.
const PSG_PORT_END: u8 = 0xA3;
/// First MSX PPI I/O port.
const PPI_PORT_START: u8 = 0xA8;
/// Last MSX PPI I/O port.
const PPI_PORT_END: u8 = 0xAB;
/// First RP5C01 address and data port.
const RTC_PORT_START: u8 = 0xB4;
/// Last S1985-mirrored RP5C01 port.
const RTC_PORT_END: u8 = 0xB7;
/// First Kanji ROM port.
const KANJI_PORT_START: u8 = 0xD8;
/// Last Kanji ROM port.
const KANJI_PORT_END: u8 = 0xDB;
/// Sony system flags port.
const SYSTEM_FLAGS_PORT: u8 = 0xF4;
/// First memory-mapper I/O port.
const MAPPER_PORT_START: u8 = 0xFC;
/// Last memory-mapper I/O port.
const MAPPER_PORT_END: u8 = 0xFF;
/// Master-clock divisor driving the YM2149.
const PSG_CLOCK_DIVISOR: u32 = 12;
/// YM2413 input clock divisor from the master clock.
const OPLL_CLOCK_DIVISOR: u32 = 6;
/// Standard SCC cartridge level after chip normalization.
const SCC_VOLUME: f32 = 8.0 / 7.0;
/// HB-F1XDJ MSX-MUSIC level after chip normalization.
const MSX_MUSIC_VOLUME: f32 = 9.0 / 28.0;
/// Panasonic FS-CA1 MSX-AUDIO level after chip normalization.
const MSX_AUDIO_VOLUME: f32 = 0.8;
/// HB-F1XDJ keyboard-click level after chip normalization.
const KEYBOARD_CLICK_VOLUME: f32 = 127.0 / 504.0;
/// Default master mix level used for MSX output calibration.
const MSX_MIX_VOLUME: f32 = 0.75;
/// Master tick at which every physical scanline is latched in HBlank.
const SCANLINE_LATCH_TICK: u64 = 1_282;
/// Master tick within the line at which vertical blank begins.
const VBLANK_LINE_TICK: u64 = 202;
/// First active line in a 192-line Japanese NTSC frame.
const ACTIVE_192_START_LINE: i16 = 35;
/// First active line in a 212-line Japanese NTSC frame.
const ACTIVE_212_START_LINE: i16 = 25;
/// Last physical line included in the visible analog surface.
const LAST_VISIBLE_LINE: u16 = 257;
/// Deterministic RTC seed used until the host clock is installed.
const INITIAL_RTC_TIME: HostDateTime = HostDateTime {
    year: 1980,
    month: 1,
    day: 1,
    day_of_week: 2,
    hour: 0,
    minute: 0,
    second: 0,
};

/// Error while installing a synthetic Z80 program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticProgramError {
    /// The program does not fit in the 64 KiB synthetic work-RAM image.
    TooLarge {
        /// Program size in bytes.
        size: usize,
    },
}

impl core::fmt::Display for SyntheticProgramError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLarge { size } => {
                write!(
                    formatter,
                    "synthetic MSX program is {size} bytes, maximum is 65536"
                )
            }
        }
    }
}

impl std::error::Error for SyntheticProgramError {}

impl MsxBus<NoTrace> {
    /// Creates an untraced bus.
    pub fn new(model: MsxModel, sample_rate: u32) -> Self {
        Self::new_with_trace_sink(model, sample_rate, NoTrace)
    }
}

/// MSX system bus.
pub struct MsxBus<T: TraceSink = NoTrace> {
    model: MsxModel,
    memory: MsxMemory,
    ppi: MsxPpi,
    keyboard: MsxKeyboard,
    psg: MsxPsg,
    fdc: Option<Wd17xxFdc<WD17XX_PLATFORM_MSX>>,
    fdc_drive_control: u8,
    rtc: Option<Rp5c01>,
    rtc_address: u8,
    s1985: Option<S1985>,
    switched_io_device: u8,
    kanji: Option<MsxKanjiRom>,
    opll: Option<OpnFm<Ym2413>>,
    msx_audio: Option<FsCa1>,
    system_flags: u8,
    vdp: MsxVdp,
    renderer: MsxRenderer,
    cassette: device::cassette::CassetteDeck,
    keyboard_click: device::one_bit_dac::OneBitDac,
    sample_rate: u32,
    clock: MsxClock,
    scheduler: MsxScheduler,
    current_cycle: u64,
    wait_cycles: i64,
    scanline: u16,
    completed_scanlines: u64,
    vblank_count: u64,
    frame_number: u64,
    tracer: T,
}

save_state::runtime_state! {
/// Complete authoritative MSX family bus state.
#[derive(Clone)]
pub(crate) struct MsxBusState {
    model: u8,
    memory: crate::memory::MsxMemoryState,
    ppi: crate::bus::ppi::MsxPpiState,
    keyboard: crate::bus::keyboard::MsxKeyboardState,
    psg: crate::bus::psg::MsxPsgState,
    fdc: Option<device::wd17xx_fdc::Wd17xxFdcState>,
    fdc_drive_control: u8,
    rtc: Option<device::rp5c01::Rp5c01State>,
    rtc_address: u8,
    s1985: Option<crate::bus::s1985::S1985State>,
    switched_io_device: u8,
    kanji: Option<crate::bus::kanji::MsxKanjiState>,
    opll: Option<device::opn_fm::OpnFmState<ymfm_oxide::Ym2413, ymfm_oxide::YmfmOutput2>>,
    msx_audio: Option<device::msx_audio::FsCa1State>,
    system_flags: u8,
    vdp: device::video_msx::MsxVdpState,
    renderer: software_renderer::MsxRendererState,
    cassette: device::cassette::CassetteDeckState,
    keyboard_click: device::one_bit_dac::OneBitDacState,
    scheduler: common::SchedulerState,
    current_cycle: u64,
    wait_cycles: i64,
    scanline: u16,
    completed_scanlines: u64,
    vblank_count: u64,
    frame_number: u64,
}}

impl<T: TraceSink> MsxBus<T> {
    /// Creates a traced bus.
    pub fn new_with_trace_sink(model: MsxModel, sample_rate: u32, tracer: T) -> Self {
        let clock = MsxClock::new(model.clock_profile());
        let mut scheduler = MsxScheduler::new();
        scheduler.schedule(
            EventMsx::Scanline,
            clock.fire_cycle_at_vdp_tick(SCANLINE_LATCH_TICK),
        );
        let fdc = match model.disk_controller() {
            MsxDiskController::None => None,
            MsxDiskController::SonyWd2793 => {
                let mut controller = Wd17xxFdc::new(model.main_clock_hz());
                controller.set_irq_enable(true);
                controller.set_double_density(true);
                Some(controller)
            }
        };
        let mut keyboard_click = device::one_bit_dac::OneBitDac::new();
        keyboard_click.configure_audio(model.main_clock_hz(), sample_rate);
        let mut bus = Self {
            model,
            memory: MsxMemory::new(model),
            ppi: MsxPpi::new(),
            keyboard: MsxKeyboard::new(model.keyboard_layout()),
            psg: MsxPsg::new(
                model.psg_keyboard_layout_bit(),
                u64::from(model.clock_profile().master_clock_hz),
                PSG_CLOCK_DIVISOR,
                model.main_clock_hz(),
                sample_rate,
            ),
            fdc,
            fdc_drive_control: 0,
            rtc: model.has_rtc().then(|| Rp5c01::new(INITIAL_RTC_TIME, 0)),
            rtc_address: 0,
            s1985: model.has_s1985().then(S1985::new),
            switched_io_device: 0,
            kanji: model.kanji_rom_size().map(MsxKanjiRom::new),
            opll: model.has_msx_music().then(|| {
                OpnFm::new(
                    model.main_clock_hz(),
                    sample_rate,
                    model.clock_profile().master_clock_hz / OPLL_CLOCK_DIVISOR,
                )
            }),
            msx_audio: None,
            system_flags: 0,
            vdp: MsxVdp::new(model.vdp_version(), model.vram_size()),
            renderer: MsxRenderer::new_for_version(model.vdp_version()),
            cassette: device::cassette::CassetteDeck::new(),
            keyboard_click,
            sample_rate,
            clock,
            scheduler,
            current_cycle: 0,
            wait_cycles: 0,
            scanline: 0,
            completed_scanlines: 0,
            vblank_count: 0,
            frame_number: 0,
            tracer,
        };
        bus.schedule_video_timing();
        bus
    }

    /// Model selected for this bus.
    pub const fn model(&self) -> MsxModel {
        self.model
    }

    /// Normal Z80 clock in Hz.
    pub const fn cpu_clock_hz(&self) -> u32 {
        self.model.main_clock_hz()
    }

    /// Sets and immediately seeds the RP5C01 from the host clock.
    pub fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        if let Some(rtc) = self.rtc.as_mut() {
            rtc.seed_time(provider(), self.current_cycle);
        }
    }

    /// Returns identities for installed firmware and cartridges.
    pub(crate) fn save_state_resources(
        &self,
    ) -> Result<save_state::ResourceManifest, save_state::StateValidationError> {
        let mut bindings = self.memory.resource_bindings()?;
        if let Some(audio) = &self.msx_audio {
            bindings.push(save_state::ResourceBinding {
                identifier: save_state::ResourceBindingId::new("firmware:msx-audio")?,
                identity: audio.resource_identity(),
            });
        }
        save_state::ResourceManifest::new(bindings)
    }

    /// Returns identities for mounted floppy and cassette media.
    pub(crate) fn save_state_media(
        &self,
    ) -> Result<save_state::MediaManifest, save_state::StateValidationError> {
        let mut bindings = match &self.fdc {
            Some(fdc) => fdc.media_manifest()?.bindings().to_vec(),
            None => Vec::new(),
        };
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

    /// Captures every authoritative mutable bus component.
    pub(crate) fn capture_runtime_state(&self) -> Result<MsxBusState, save_state::SaveStateError> {
        Ok(MsxBusState {
            model: match self.model {
                MsxModel::Msx => 0,
                MsxModel::Msx2 => 1,
                MsxModel::Msx2Plus => 2,
            },
            memory: self.memory.capture_state(),
            ppi: self.ppi.capture_state(),
            keyboard: self.keyboard.capture_state(),
            psg: self.psg.capture_state(),
            fdc: self
                .fdc
                .as_ref()
                .map(Wd17xxFdc::capture_state)
                .transpose()?,
            fdc_drive_control: self.fdc_drive_control,
            rtc: self.rtc.as_ref().map(Rp5c01::capture_state),
            rtc_address: self.rtc_address,
            s1985: self.s1985.as_ref().map(S1985::capture_state),
            switched_io_device: self.switched_io_device,
            kanji: self.kanji.as_ref().map(MsxKanjiRom::capture_state),
            opll: self.opll.as_ref().map(OpnFm::capture_state),
            msx_audio: self.msx_audio.as_ref().map(FsCa1::capture_state),
            system_flags: self.system_flags,
            vdp: self.vdp.capture_state(),
            renderer: self.renderer.capture_state(),
            cassette: self.cassette.capture_state(),
            keyboard_click: self.keyboard_click.capture_state(),
            scheduler: self.scheduler.capture_state(),
            current_cycle: self.current_cycle,
            wait_cycles: self.wait_cycles,
            scanline: self.scanline,
            completed_scanlines: self.completed_scanlines,
            vblank_count: self.vblank_count,
            frame_number: self.frame_number,
        })
    }

    /// Validates and restores every authoritative mutable bus component.
    pub(crate) fn restore_runtime_state(
        &mut self,
        state: MsxBusState,
    ) -> Result<(), save_state::SaveStateError> {
        let model = match state.model {
            0 => MsxModel::Msx,
            1 => MsxModel::Msx2,
            2 => MsxModel::Msx2Plus,
            _ => {
                return Err(
                    save_state::StateValidationError::new("MSX model state is invalid").into(),
                );
            }
        };
        if model != self.model || state.scanline >= NTSC_TOTAL_SCANLINES || state.rtc_address > 0x0F
        {
            return Err(
                save_state::StateValidationError::new("MSX timing configuration differs").into(),
            );
        }
        match (&mut self.fdc, state.fdc) {
            (Some(fdc), Some(fdc_state)) => fdc.restore_state(fdc_state)?,
            (None, None) => {}
            _ => {
                return Err(
                    save_state::StateValidationError::new("MSX FDC configuration differs").into(),
                );
            }
        }
        match (&mut self.rtc, state.rtc) {
            (Some(rtc), Some(rtc_state)) => rtc.restore_state(rtc_state)?,
            (None, None) => {}
            _ => {
                return Err(
                    save_state::StateValidationError::new("MSX RTC configuration differs").into(),
                );
            }
        }
        match (&mut self.s1985, state.s1985) {
            (Some(controller), Some(controller_state)) => {
                controller.restore_state(controller_state)?
            }
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX S1985 configuration differs",
                )
                .into());
            }
        }
        match (&mut self.kanji, state.kanji) {
            (Some(kanji), Some(kanji_state)) => kanji.restore_state(kanji_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX Kanji configuration differs",
                )
                .into());
            }
        }
        match (&mut self.opll, state.opll) {
            (Some(opll), Some(opll_state)) => opll.restore_state(opll_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX OPLL configuration differs",
                )
                .into());
            }
        }
        match (&mut self.msx_audio, state.msx_audio) {
            (Some(audio), Some(audio_state)) => audio.restore_state(audio_state)?,
            (None, None) => {}
            _ => {
                return Err(save_state::StateValidationError::new(
                    "MSX-AUDIO configuration differs",
                )
                .into());
            }
        }
        self.memory.restore_state(state.memory)?;
        self.ppi.restore_state(state.ppi)?;
        self.keyboard.restore_state(state.keyboard);
        self.psg.restore_state(state.psg)?;
        self.vdp.restore_state(state.vdp)?;
        self.renderer.restore_state(state.renderer)?;
        self.cassette.restore_state(state.cassette)?;
        self.keyboard_click.restore_state(state.keyboard_click)?;
        self.scheduler.restore_state(state.scheduler)?;
        self.fdc_drive_control = state.fdc_drive_control;
        self.rtc_address = state.rtc_address;
        self.switched_io_device = state.switched_io_device;
        self.system_flags = state.system_flags;
        self.current_cycle = state.current_cycle;
        self.wait_cycles = state.wait_cycles;
        self.scanline = state.scanline;
        self.completed_scanlines = state.completed_scanlines;
        self.vblank_count = state.vblank_count;
        self.frame_number = state.frame_number;
        Ok(())
    }

    /// Installs hash-validated firmware for the selected model.
    pub fn load_firmware(&mut self, firmware: &LoadedFirmware) -> Result<(), FirmwareInstallError> {
        self.memory.load_firmware(firmware)?;
        if let Some(kanji) = self.kanji.as_mut()
            && let Some(region) = firmware.region(crate::FirmwareRegion::KanjiFont)
        {
            kanji.load(region.bytes());
        }
        if let Some(opll) = self.opll.as_mut()
            && let Some(region) = firmware.region(crate::FirmwareRegion::OpllInstruments)
        {
            let instruments: &[u8; YM2413_INSTRUMENT_DATA_SIZE] = region
                .bytes()
                .try_into()
                .expect("validated YM2413 instrument region size");
            opll.chip_mut().set_instrument_data(instruments);
        }
        self.msx_audio = firmware
            .msx_audio()
            .and_then(|bytes| FsCa1::new(bytes, self.cpu_clock_hz(), self.sample_rate));
        Ok(())
    }

    /// Identifies and inserts a cartridge into connector zero or one.
    pub fn insert_cartridge(
        &mut self,
        slot: usize,
        image: &[u8],
    ) -> Result<CartridgeLoadInfo, CartridgeError> {
        let info = self
            .memory
            .insert_cartridge(slot, image, self.current_cycle)?;
        self.memory
            .configure_cartridge_audio(slot, self.cpu_clock_hz(), self.sample_rate);
        Ok(info)
    }

    /// Identifies and inserts a file-backed cartridge.
    pub fn insert_cartridge_from_path(
        &mut self,
        slot: usize,
        image: &[u8],
        path: &Path,
    ) -> Result<CartridgeLoadInfo, CartridgeError> {
        let info = self
            .memory
            .insert_cartridge_from_path(slot, image, path, self.current_cycle)?;
        self.memory
            .configure_cartridge_audio(slot, self.cpu_clock_hz(), self.sample_rate);
        Ok(info)
    }

    /// Ejects the cartridge in connector zero or one.
    pub fn eject_cartridge(&mut self, slot: usize) -> Result<(), CartridgeError> {
        self.memory.eject_cartridge(slot)
    }

    /// Flushes dirty battery-backed data for all cartridges.
    pub fn flush_cartridges(&mut self) -> Result<(), CartridgeError> {
        self.memory.flush_cartridges()
    }

    /// Inserts a sample-level cassette signal.
    pub fn insert_cassette_signal(&mut self, signal: device::cassette::SampledSignal) {
        self.cassette.insert_signal(signal);
        self.cassette.set_motor(self.ppi.cassette_motor());
    }

    /// Parses and inserts a file-backed MSX cassette.
    pub fn insert_cassette_from_path(
        &mut self,
        extension: &str,
        image: &[u8],
        path: &Path,
    ) -> Result<(), crate::MsxCassetteError> {
        let signal = crate::load_msx_cassette(extension, image)?;
        self.cassette
            .insert_media_from_path(device::cassette::CassetteMedia::Samples(signal), path);
        self.cassette
            .advance(self.current_cycle, self.cpu_clock_hz());
        self.cassette.set_motor(self.ppi.cassette_motor());
        Ok(())
    }

    /// Ejects the cassette.
    pub fn eject_cassette(&mut self) {
        self.cassette.eject();
    }

    /// Connects a device to controller port zero or one.
    pub fn set_controller(&mut self, port: usize, device: MsxControllerDevice) {
        self.psg.set_controller(port, device);
    }

    /// Updates a joystick on controller port zero or one.
    pub fn set_joystick(&mut self, port: usize, state: MsxJoystickState) {
        self.psg.set_joystick(port, state);
    }

    /// Accumulates host mouse movement on controller port A.
    pub fn push_mouse_delta(&mut self, delta_x: i16, delta_y: i16) {
        self.psg.push_mouse_delta(delta_x, delta_y);
    }

    /// Updates mouse buttons on controller port A.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool) {
        self.psg.set_mouse_buttons(left, right);
    }

    /// Applies one host keyboard make or break scancode.
    pub fn push_keyboard_scancode(&mut self, code: u8) {
        self.keyboard.push_scancode(code);
    }

    /// Whether the cassette motor is on.
    pub const fn cassette_motor_on(&self) -> bool {
        self.ppi.cassette_motor()
    }

    /// Current cassette output level.
    pub const fn cassette_output_high(&self) -> bool {
        self.ppi.cassette_output()
    }

    /// Whether the Caps LED is lit.
    pub const fn caps_led_on(&self) -> bool {
        self.ppi.caps_led()
    }

    /// Whether the Kana LED is lit.
    pub const fn kana_led_on(&self) -> bool {
        self.psg.kana_led()
    }

    /// Whether internal Panasonic FS-CA1 hardware is installed.
    pub const fn has_msx_audio(&self) -> bool {
        self.msx_audio.is_some()
    }

    /// Generates YM2149, YM2413, SCC and keyboard-click audio.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let volume = volume * MSX_MIX_VOLUME;
        let written = self.psg.generate_samples(
            self.current_cycle,
            u64::from(self.model.clock_profile().master_clock_hz),
            PSG_CLOCK_DIVISOR,
            self.cpu_clock_hz(),
            self.sample_rate,
            volume,
            output,
        );
        let scc_written = self.memory.mix_scc_samples(
            self.current_cycle,
            self.cpu_clock_hz(),
            self.sample_rate,
            volume * SCC_VOLUME,
            output,
        );
        let cpu_clock_hz = self.model.main_clock_hz();
        if let Some(opll) = self.opll.as_mut() {
            opll.generate_samples(
                self.current_cycle,
                cpu_clock_hz,
                volume * MSX_MUSIC_VOLUME,
                output,
            );
        }
        if let Some(msx_audio) = self.msx_audio.as_mut() {
            msx_audio.generate_samples(
                self.current_cycle,
                cpu_clock_hz,
                volume * MSX_AUDIO_VOLUME,
                output,
            );
        }
        let click_written = self.keyboard_click.mix_samples(
            self.current_cycle,
            self.cpu_clock_hz(),
            self.sample_rate,
            volume * KEYBOARD_CLICK_VOLUME,
            output,
        );
        written.max(scc_written).max(click_written)
    }

    /// Installs a test-owned Z80 program at address zero.
    pub fn load_synthetic_program(&mut self, program: &[u8]) -> Result<(), SyntheticProgramError> {
        if program.len() > SYNTHETIC_ADDRESS_SPACE_SIZE {
            return Err(SyntheticProgramError::TooLarge {
                size: program.len(),
            });
        }
        self.memory.install_synthetic_program(program);
        self.ppi
            .select_primary_slots_for_synthetic_program(self.memory.primary_slot_register());
        Ok(())
    }

    /// Reads the currently selected memory without tracing.
    pub fn peek_byte(&self, address: u16) -> u8 {
        self.memory.read(address).value
    }

    /// Writes the currently selected memory without tracing.
    pub fn poke_byte(&mut self, address: u16, value: u8) {
        if self.fdc_is_selected(address) {
            self.fdc_write(address, value);
        } else {
            self.memory.write_at(address, value, self.current_cycle);
        }
    }

    /// Reads memory or a selected memory-mapped disk register.
    fn read_memory(&mut self, address: u16) -> (u8, bool) {
        if self.fdc_is_selected(address) {
            (self.fdc_read(address), true)
        } else if self.memory.internal_expansion_selected(address)
            && let Some(msx_audio) = self.msx_audio.as_ref()
        {
            (msx_audio.read_memory(address), true)
        } else {
            let read = self.memory.read_at(address, self.current_cycle);
            (read.value, read.handled)
        }
    }

    /// Writes memory or a selected memory-mapped disk register.
    fn write_memory(&mut self, address: u16, value: u8) -> (bool, Option<SecondarySlotChange>) {
        if self.fdc_is_selected(address) {
            self.fdc_write(address, value);
            (true, None)
        } else if self.memory.internal_expansion_selected(address)
            && let Some(msx_audio) = self.msx_audio.as_mut()
        {
            msx_audio.write_memory(address, value);
            (true, None)
        } else {
            let write = self.memory.write_at(address, value, self.current_cycle);
            (write.handled, write.secondary_change)
        }
    }

    /// Current monotonic Z80 cycle.
    pub const fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Advances the monotonic Z80 cycle.
    pub fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
    }

    /// Current VDP master-clock tick.
    pub fn vdp_tick(&self) -> u64 {
        self.clock.vdp_tick_at(self.current_cycle)
    }

    /// Current whole VDP base dot.
    pub fn vdp_dot(&self) -> u64 {
        self.clock.vdp_dot_at(self.current_cycle).dot
    }

    /// Master-clock phase within the current VDP base dot.
    pub fn vdp_dot_phase(&self) -> u8 {
        self.clock.vdp_dot_at(self.current_cycle).phase
    }

    /// Current Japanese NTSC scanline.
    pub const fn scanline(&self) -> u16 {
        self.scanline
    }

    /// Number of scanline events processed since construction.
    pub const fn completed_scanlines(&self) -> u64 {
        self.completed_scanlines
    }

    /// Number of physically visible frames presented.
    pub const fn frame_number(&self) -> u64 {
        self.frame_number
    }

    /// Current VDP status without read side effects.
    pub const fn vdp_status(&self) -> u8 {
        self.vdp.status()
    }

    /// Current render-visible VDP register and palette state.
    pub const fn vdp_render_state(&self) -> MsxVdpRenderState {
        self.vdp.render_state()
    }

    /// Complete physical VDP RAM.
    pub fn vdp_vram(&self) -> &[u8] {
        self.vdp.vram()
    }

    /// Whether the VDP is asserting the Z80 interrupt line.
    pub fn has_irq(&self) -> bool {
        self.vdp.irq_pending() || self.msx_audio.as_ref().is_some_and(FsCa1::irq_asserted)
    }

    /// Last presented packed RGBA framebuffer.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// Physical visible-signal framebuffer dimensions.
    pub const fn display_dimensions(&self) -> (u32, u32) {
        self.renderer.dimensions()
    }

    /// Cycle of the next scheduled event.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Immutable access to the tracer.
    pub const fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Mutable access to the tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Processes every event due at the current cycle.
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
                EventMsx::Scanline => {
                    self.synchronize_vdp();
                    let physical_line = self.scanline;
                    let status = self.renderer.latch_scanline(
                        &RenderInputsMsx {
                            vram: self.vdp.vram(),
                            state: self.vdp.render_state(),
                        },
                        physical_line,
                    );
                    self.vdp.merge_sprite_status(status);
                    if physical_line == LAST_VISIBLE_LINE {
                        self.present_latched_frame();
                    }
                    self.scanline = (self.scanline + 1) % NTSC_TOTAL_SCANLINES;
                    if self.scanline == 0 {
                        self.vdp.start_frame();
                        self.schedule_video_timing();
                    }
                    self.completed_scanlines = self.completed_scanlines.wrapping_add(1);
                    let next_tick = self
                        .completed_scanlines
                        .saturating_mul(crate::clock::VDP_TICKS_PER_SCANLINE)
                        .saturating_add(SCANLINE_LATCH_TICK);
                    self.scheduler.schedule(
                        EventMsx::Scanline,
                        self.clock.fire_cycle_at_vdp_tick(next_tick),
                    );
                }
                EventMsx::VBlank => {
                    self.synchronize_vdp();
                    self.vdp.enter_vertical_blank();
                    self.vblank_count = self.vblank_count.wrapping_add(1);
                    self.schedule_video_timing();
                }
                EventMsx::LineInterrupt => {
                    self.synchronize_vdp();
                    self.vdp.enter_line_interrupt();
                    self.schedule_video_timing();
                }
                EventMsx::FdcTask => self.run_fdc_task(self.current_cycle),
                EventMsx::FdcPio => self.run_fdc_pio(self.current_cycle),
                EventMsx::Y8950TimerA | EventMsx::Y8950TimerB => {
                    let timer_id = u32::from(event.kind == EventMsx::Y8950TimerB);
                    if let Some(msx_audio) = self.msx_audio.as_mut() {
                        msx_audio.timer_expired(timer_id, self.current_cycle);
                        let _ = msx_audio.take_irq_change();
                    }
                    self.apply_msx_audio_timers();
                }
            }
        }
    }

    fn read_io(&mut self, port: u16) -> (u8, bool) {
        let raw_port = port as u8;
        let low_port = self.normalize_io_port(raw_port);
        match low_port {
            SWITCHED_IO_PORT_START..=SWITCHED_IO_PORT_END => {
                if self.switched_io_device == S1985_DEVICE_ID {
                    self.s1985.as_mut().map_or((OPEN_BUS, false), |controller| {
                        (controller.read(low_port - SWITCHED_IO_PORT_START), true)
                    })
                } else {
                    (OPEN_BUS, self.s1985.is_some())
                }
            }
            OPLL_ADDRESS_PORT | OPLL_DATA_PORT => (OPEN_BUS, self.opll.is_some()),
            PRINTER_PORT_START..=PRINTER_PORT_END if matches!(self.model, MsxModel::Msx2Plus) => {
                if (raw_port - PRINTER_PORT_START) & 0x03 == 0 {
                    (0x02, true)
                } else {
                    (OPEN_BUS, true)
                }
            }
            VDP_DATA_PORT => {
                self.synchronize_vdp();
                (self.vdp.timed_data_read(), true)
            }
            VDP_CONTROL_PORT => {
                self.synchronize_vdp();
                (self.vdp.status_read(), true)
            }
            VDP_PALETTE_PORT | VDP_INDIRECT_PORT => (OPEN_BUS, self.model.vdp_version().is_v99x8()),
            PSG_PORT_START..=PSG_PORT_END => match low_port - PSG_PORT_START {
                2 => {
                    self.cassette
                        .advance(self.current_cycle, self.cpu_clock_hz());
                    (self.psg.data_read(self.cassette.ear_level()), true)
                }
                0 | 1 => (OPEN_BUS, true),
                _ => (OPEN_BUS, false),
            },
            PPI_PORT_START..=PPI_PORT_END => {
                let keyboard_row = self.keyboard.row(self.ppi.selected_row());
                (self.ppi.read(low_port - PPI_PORT_START, keyboard_row), true)
            }
            RTC_PORT_START..=RTC_PORT_END => {
                let Some(rtc) = self.rtc.as_mut() else {
                    return (OPEN_BUS, false);
                };
                if (low_port - RTC_PORT_START) & 1 == 0 {
                    (OPEN_BUS, true)
                } else {
                    (
                        0xF0 | rtc.read(
                            self.rtc_address,
                            self.current_cycle,
                            self.model.main_clock_hz(),
                        ),
                        true,
                    )
                }
            }
            0xC0..=0xC3 => self.msx_audio.as_mut().map_or((OPEN_BUS, false), |audio| {
                (
                    audio
                        .read_io(low_port, self.current_cycle)
                        .unwrap_or(OPEN_BUS),
                    true,
                )
            }),
            KANJI_PORT_START..=KANJI_PORT_END => {
                let Some(kanji) = self.kanji.as_mut() else {
                    return (OPEN_BUS, false);
                };
                if low_port & 1 != 0 {
                    (kanji.read_data((low_port - KANJI_PORT_START) >> 1), true)
                } else {
                    (OPEN_BUS, true)
                }
            }
            SYSTEM_FLAGS_PORT if matches!(self.model, MsxModel::Msx2Plus) => {
                (self.system_flags, true)
            }
            MAPPER_PORT_START..=MAPPER_PORT_END => self
                .memory
                .mapper_read(usize::from(low_port - MAPPER_PORT_START))
                .map_or((OPEN_BUS, false), |value| (value, true)),
            _ => (OPEN_BUS, false),
        }
    }

    fn write_io(&mut self, port: u16, value: u8) -> (bool, IoWriteEffect) {
        let raw_port = port as u8;
        let low_port = self.normalize_io_port(raw_port);
        match low_port {
            SWITCHED_IO_PORT_START => {
                self.switched_io_device = value;
                (self.s1985.is_some(), IoWriteEffect::None)
            }
            SWITCHED_IO_DEVICE_PORT_START..=SWITCHED_IO_PORT_END => {
                if self.switched_io_device == S1985_DEVICE_ID {
                    self.s1985
                        .as_mut()
                        .map_or((false, IoWriteEffect::None), |controller| {
                            controller.write(low_port - SWITCHED_IO_PORT_START, value);
                            (true, IoWriteEffect::None)
                        })
                } else {
                    (self.s1985.is_some(), IoWriteEffect::None)
                }
            }
            OPLL_ADDRESS_PORT => {
                let cartridge_handled =
                    self.memory
                        .fm_pac_io_write(low_port, value, self.current_cycle);
                if let Some(opll) = self.opll.as_mut() {
                    opll.write_address(value, self.current_cycle);
                }
                (
                    self.opll.is_some() || cartridge_handled,
                    IoWriteEffect::None,
                )
            }
            OPLL_DATA_PORT => {
                let cartridge_handled =
                    self.memory
                        .fm_pac_io_write(low_port, value, self.current_cycle);
                if let Some(opll) = self.opll.as_mut() {
                    opll.write_data(value, self.current_cycle);
                }
                (
                    self.opll.is_some() || cartridge_handled,
                    IoWriteEffect::None,
                )
            }
            PRINTER_PORT_START..=PRINTER_PORT_END if matches!(self.model, MsxModel::Msx2Plus) => {
                (true, IoWriteEffect::None)
            }
            VDP_DATA_PORT => {
                self.synchronize_vdp();
                self.vdp.timed_data_write(value);
                (true, IoWriteEffect::None)
            }
            VDP_CONTROL_PORT => {
                self.synchronize_vdp();
                let effects = self.vdp.timed_control_write(value);
                self.apply_vdp_effects(effects);
                (true, IoWriteEffect::None)
            }
            VDP_PALETTE_PORT if self.model.vdp_version().is_v99x8() => {
                self.synchronize_vdp();
                self.vdp.palette_write(value);
                (true, IoWriteEffect::None)
            }
            VDP_INDIRECT_PORT if self.model.vdp_version().is_v99x8() => {
                self.synchronize_vdp();
                let effects = self.vdp.indirect_write(value);
                self.apply_vdp_effects(effects);
                (true, IoWriteEffect::None)
            }
            PSG_PORT_START => {
                self.psg.address_write(value);
                (true, IoWriteEffect::None)
            }
            port if port == PSG_PORT_START + 1 => {
                self.psg.data_write(value, self.current_cycle);
                (true, IoWriteEffect::None)
            }
            PPI_PORT_START..=PPI_PORT_END => {
                self.cassette
                    .advance(self.current_cycle, self.cpu_clock_hz());
                let effect = self.ppi.write(low_port - PPI_PORT_START, value);
                if effect.motor_changed {
                    self.cassette.set_motor(self.ppi.cassette_motor());
                }
                if effect.click_changed {
                    self.keyboard_click
                        .set_high(self.ppi.keyboard_click(), self.current_cycle);
                }
                if let Some(primary_slots) = effect.primary_slots {
                    self.memory.set_primary_slot_register(primary_slots);
                    (true, IoWriteEffect::PrimarySlots)
                } else {
                    (true, IoWriteEffect::None)
                }
            }
            RTC_PORT_START..=RTC_PORT_END => {
                let Some(rtc) = self.rtc.as_mut() else {
                    return (false, IoWriteEffect::None);
                };
                if (low_port - RTC_PORT_START) & 1 == 0 {
                    self.rtc_address = value & 0x0F;
                } else {
                    rtc.write(
                        self.rtc_address,
                        value,
                        self.current_cycle,
                        self.model.main_clock_hz(),
                    );
                }
                (true, IoWriteEffect::None)
            }
            0xC0..=0xC3 => {
                let Some(msx_audio) = self.msx_audio.as_mut() else {
                    return (false, IoWriteEffect::None);
                };
                let _ = msx_audio.write_io(low_port, value, self.current_cycle);
                let _ = msx_audio.take_irq_change();
                self.apply_msx_audio_timers();
                (true, IoWriteEffect::None)
            }
            KANJI_PORT_START..=KANJI_PORT_END => {
                let Some(kanji) = self.kanji.as_mut() else {
                    return (false, IoWriteEffect::None);
                };
                kanji.write_address(low_port, value);
                (true, IoWriteEffect::None)
            }
            SYSTEM_FLAGS_PORT if matches!(self.model, MsxModel::Msx2Plus) => {
                self.system_flags = (self.system_flags & 0x20) | (value & 0xA0);
                (true, IoWriteEffect::None)
            }
            MAPPER_PORT_START..=MAPPER_PORT_END => {
                let page = usize::from(low_port - MAPPER_PORT_START);
                self.memory.mapper_write(page, value).map_or(
                    (false, IoWriteEffect::None),
                    |write| {
                        (
                            true,
                            IoWriteEffect::Mapper {
                                page: write.page,
                                value: write.value,
                                physical_segment: write.physical_segment,
                            },
                        )
                    },
                )
            }
            _ => (false, IoWriteEffect::None),
        }
    }

    /// Applies the HB-F1XDJ four-port device mirrors.
    fn normalize_io_port(&self, port: u8) -> u8 {
        if !matches!(self.model, MsxModel::Msx2Plus) {
            return port;
        }
        match port {
            0x98..=0x9F => VDP_DATA_PORT + (port & 0x03),
            0xA0..=0xA7 => PSG_PORT_START + (port & 0x03),
            0xA8..=0xAF => PPI_PORT_START + (port & 0x03),
            _ => port,
        }
    }

    /// Advances asynchronous VDP work to the current CPU boundary.
    fn synchronize_vdp(&mut self) {
        let tick = self.clock.vdp_tick_at(self.current_cycle);
        let frame_ticks = u64::from(NTSC_TOTAL_SCANLINES) * crate::clock::VDP_TICKS_PER_SCANLINE;
        let line = ((tick % frame_ticks) / crate::clock::VDP_TICKS_PER_SCANLINE) as u16;
        let active_start = self.active_start_line();
        let active_end = active_start.saturating_add(self.vdp.active_lines());
        let display_active =
            self.vdp.display_enabled() && (active_start..active_end).contains(&line);
        self.vdp.advance_to(tick, display_active);
    }

    /// Applies Y8950 timer changes to the machine scheduler.
    fn apply_msx_audio_timers(&mut self) {
        let actions = self
            .msx_audio
            .as_mut()
            .map_or_else(Vec::new, |audio| audio.drain_timers().to_vec());
        for action in actions {
            let (timer_id, fire_cycle) = match action {
                FmTimerAction::Schedule {
                    timer_id,
                    fire_cycle,
                } => (timer_id, Some(fire_cycle)),
                FmTimerAction::Cancel { timer_id } => (timer_id, None),
            };
            let kind = if timer_id == 0 {
                EventMsx::Y8950TimerA
            } else {
                EventMsx::Y8950TimerB
            };
            if let Some(fire_cycle) = fire_cycle {
                self.scheduler.schedule(kind, fire_cycle);
            } else {
                self.scheduler.cancel(kind);
            }
        }
    }

    /// Applies VDP access effects to machine-owned event scheduling.
    fn apply_vdp_effects(&mut self, effects: MsxVdpEffects) {
        if effects.timing_changed {
            self.schedule_video_timing();
        }
    }

    /// Returns the first active physical line after vertical adjustment.
    fn active_start_line(&self) -> u16 {
        let base = if self.vdp.active_lines() == 212 {
            ACTIVE_212_START_LINE
        } else {
            ACTIVE_192_START_LINE
        };
        (base + i16::from(self.vdp.vertical_adjust())).clamp(0, 261) as u16
    }

    /// Schedules the next VBlank and programmable line interrupt.
    fn schedule_video_timing(&mut self) {
        let current_tick = self.clock.vdp_tick_at(self.current_cycle);
        let frame_ticks = u64::from(NTSC_TOTAL_SCANLINES) * crate::clock::VDP_TICKS_PER_SCANLINE;
        let active_start = self.active_start_line();
        let vblank_line = active_start.saturating_add(self.vdp.active_lines());
        let vblank_phase =
            u64::from(vblank_line) * crate::clock::VDP_TICKS_PER_SCANLINE + VBLANK_LINE_TICK;
        let vblank_tick = next_frame_phase(current_tick, frame_ticks, vblank_phase);
        self.scheduler.schedule(
            EventMsx::VBlank,
            self.clock.fire_cycle_at_vdp_tick(vblank_tick),
        );

        if self.vdp.version().is_v99x8() && self.vdp.line_interrupt_enabled() {
            let display_line = self
                .vdp
                .interrupt_line()
                .wrapping_sub(self.vdp.vertical_scroll());
            let physical_line = (u32::from(active_start) + u32::from(display_line))
                % u32::from(NTSC_TOTAL_SCANLINES);
            let phase = u64::from(physical_line) * crate::clock::VDP_TICKS_PER_SCANLINE
                + SCANLINE_LATCH_TICK;
            let tick = next_frame_phase(current_tick, frame_ticks, phase);
            self.scheduler.schedule(
                EventMsx::LineInterrupt,
                self.clock.fire_cycle_at_vdp_tick(tick),
            );
        } else {
            self.scheduler.cancel(EventMsx::LineInterrupt);
        }
    }

    fn trace_primary_slots(&mut self) {
        for page in 0..4 {
            let primary = self.memory.primary_slot_for_page(page);
            let secondary = self.memory.secondary_slot_for_page(primary, page);
            self.trace_slot_selection(page, primary, secondary);
        }
    }

    fn trace_secondary_slots(&mut self, change: SecondarySlotChange) {
        for page in 0..4 {
            let secondary = (change.value >> (page * 2)) & 0x03;
            self.trace_slot_selection(page, change.primary, Some(secondary));
        }
    }

    fn trace_slot_selection(&mut self, page: usize, primary: u8, secondary: Option<u8>) {
        if !T::ENABLED
            || !self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::MSX_SLOT,
                action: trace_id::action::SELECT,
            })
        {
            return;
        }
        let context =
            TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz())));
        if let Some(secondary) = secondary {
            self.tracer.trace(
                context,
                TraceEvent::Device(TraceDeviceEvent {
                    device: trace_id::device::MSX_SLOT,
                    action: trace_id::action::SELECT,
                    fields: &[
                        TraceField {
                            name: trace_id::field::PAGE,
                            value: TraceValue::Unsigned(page as u64),
                        },
                        TraceField {
                            name: trace_id::field::PRIMARY_SLOT,
                            value: TraceValue::Unsigned(u64::from(primary)),
                        },
                        TraceField {
                            name: trace_id::field::SECONDARY_SLOT,
                            value: TraceValue::Unsigned(u64::from(secondary)),
                        },
                    ],
                }),
            );
        } else {
            self.tracer.trace(
                context,
                TraceEvent::Device(TraceDeviceEvent {
                    device: trace_id::device::MSX_SLOT,
                    action: trace_id::action::SELECT,
                    fields: &[
                        TraceField {
                            name: trace_id::field::PAGE,
                            value: TraceValue::Unsigned(page as u64),
                        },
                        TraceField {
                            name: trace_id::field::PRIMARY_SLOT,
                            value: TraceValue::Unsigned(u64::from(primary)),
                        },
                    ],
                }),
            );
        }
    }

    fn trace_mapper_bank(&mut self, page: usize, value: u8, physical_segment: Option<u8>) {
        if T::ENABLED
            && self.tracer.interested(TraceEventKey::Device {
                device: trace_id::device::MSX_MAPPER,
                action: trace_id::action::BANK,
            })
        {
            let context =
                TraceContext::main_cpu(self.current_cycle, Some(u64::from(self.cpu_clock_hz())));
            if let Some(physical_segment) = physical_segment {
                self.tracer.trace(
                    context,
                    TraceEvent::Device(TraceDeviceEvent {
                        device: trace_id::device::MSX_MAPPER,
                        action: trace_id::action::BANK,
                        fields: &[
                            TraceField {
                                name: trace_id::field::PAGE,
                                value: TraceValue::Unsigned(page as u64),
                            },
                            TraceField {
                                name: trace_id::field::VALUE,
                                value: TraceValue::Unsigned(u64::from(value)),
                            },
                            TraceField {
                                name: trace_id::field::SEGMENT,
                                value: TraceValue::Unsigned(u64::from(physical_segment)),
                            },
                        ],
                    }),
                );
            } else {
                self.tracer.trace(
                    context,
                    TraceEvent::Device(TraceDeviceEvent {
                        device: trace_id::device::MSX_MAPPER,
                        action: trace_id::action::BANK,
                        fields: &[
                            TraceField {
                                name: trace_id::field::PAGE,
                                value: TraceValue::Unsigned(page as u64),
                            },
                            TraceField {
                                name: trace_id::field::VALUE,
                                value: TraceValue::Unsigned(u64::from(value)),
                            },
                        ],
                    }),
                );
            }
        }
    }

    /// Presents and clears the completed visible-signal frame.
    fn present_latched_frame(&mut self) {
        let (width, height) = self.renderer.present_latched_frame();
        self.frame_number = self.frame_number.wrapping_add(1);
        if T::ENABLED {
            self.tracer.trace(
                TraceContext::presentation_main(
                    self.current_cycle,
                    Some(u64::from(self.cpu_clock_hz())),
                ),
                TraceEvent::Presentation(TracePresentation {
                    display: trace_id::display::MAIN,
                    frame: self.frame_number,
                    width,
                    height,
                }),
            );
        }
        self.renderer.clear_latched_frame();
    }
}

/// Returns the next occurrence of one phase in a repeating frame.
fn next_frame_phase(current_tick: u64, frame_ticks: u64, phase: u64) -> u64 {
    let frame_start = current_tick / frame_ticks * frame_ticks;
    let candidate = frame_start.saturating_add(phase);
    if candidate > current_tick {
        candidate
    } else {
        candidate.saturating_add(frame_ticks)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IoWriteEffect {
    None,
    PrimarySlots,
    Mapper {
        page: usize,
        value: u8,
        physical_segment: Option<u8>,
    },
}

/// Ephemeral `common::Bus` adapter for the main Z80.
pub struct MainBusView<'a, T: TraceSink = NoTrace> {
    /// Shared MSX bus.
    pub bus: &'a mut MsxBus<T>,
}

impl<T: TraceSink> common::Bus for MainBusView<'_, T> {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        let (value, handled) = self.bus.read_memory(address);
        self.trace_access(
            TraceAddressSpace::MAIN_MEMORY,
            TraceAccessKind::Read,
            u64::from(address),
            value,
            handled,
        );
        value
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address as u16;
        let (handled, secondary_change) = self.bus.write_memory(address, value);
        self.trace_access(
            TraceAddressSpace::MAIN_MEMORY,
            TraceAccessKind::Write,
            u64::from(address),
            value,
            handled,
        );
        if let Some(change) = secondary_change {
            self.bus.trace_secondary_slots(change);
        }
    }

    fn fetch_opcode_byte(&mut self, address: u32) -> u8 {
        let address = address as u16;
        self.bus.wait_cycles += M1_WAIT_CYCLES;
        let (value, handled) = self.bus.read_memory(address);
        self.trace_access(
            TraceAddressSpace::MAIN_MEMORY,
            TraceAccessKind::Fetch,
            u64::from(address),
            value,
            handled,
        );
        value
    }

    fn drain_wait_cycles(&mut self) -> i64 {
        core::mem::take(&mut self.bus.wait_cycles)
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        let (value, handled) = self.bus.read_io(port);
        self.trace_access(
            TraceAddressSpace::MAIN_IO,
            TraceAccessKind::Read,
            u64::from(port),
            value,
            handled,
        );
        value
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        let (handled, effect) = self.bus.write_io(port, value);
        self.trace_access(
            TraceAddressSpace::MAIN_IO,
            TraceAccessKind::Write,
            u64::from(port),
            value,
            handled,
        );
        match effect {
            IoWriteEffect::None => {}
            IoWriteEffect::PrimarySlots => self.bus.trace_primary_slots(),
            IoWriteEffect::Mapper {
                page,
                value,
                physical_segment,
            } => self.bus.trace_mapper_bank(page, value, physical_segment),
        }
    }

    fn has_irq(&self) -> bool {
        self.bus.has_irq()
    }

    fn acknowledge_irq(&mut self) -> u8 {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::interrupt(
                    trace_id::controller::MSX_IRQ,
                    TraceInterruptKind::Maskable,
                    Some(0),
                    TraceInterruptAction::Acknowledge,
                    Some(0xFF),
                ),
            );
        }
        0xFF
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

impl<T: TraceSink> MainBusView<'_, T> {
    fn trace_access(
        &mut self,
        space: TraceAddressSpace,
        kind: TraceAccessKind,
        address: u64,
        value: u8,
        handled: bool,
    ) {
        if T::ENABLED {
            self.bus.tracer.trace(
                TraceContext::main_cpu(
                    self.bus.current_cycle,
                    Some(u64::from(self.bus.cpu_clock_hz())),
                ),
                TraceEvent::access(
                    space,
                    kind,
                    address,
                    TraceAccessWidth::Byte,
                    Some(u64::from(value)),
                    handled,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use common::Bus as _;

    use super::*;

    #[test]
    fn synthetic_program_bounds_are_checked() {
        let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
        assert!(
            bus.load_synthetic_program(&vec![0; SYNTHETIC_ADDRESS_SPACE_SIZE])
                .is_ok()
        );
        assert_eq!(
            bus.load_synthetic_program(&vec![0; SYNTHETIC_ADDRESS_SPACE_SIZE + 1]),
            Err(SyntheticProgramError::TooLarge {
                size: SYNTHETIC_ADDRESS_SPACE_SIZE + 1
            })
        );
    }

    #[test]
    fn unimplemented_io_is_open_bus() {
        let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
        let mut view = MainBusView { bus: &mut bus };
        assert_eq!(view.io_read_byte(0x80), OPEN_BUS);
        view.io_write_byte(0x80, 0x55);
    }

    #[test]
    fn only_m1_fetches_add_the_standard_wait() {
        let mut bus = MsxBus::new(MsxModel::Msx, 48_000);
        let mut view = MainBusView { bus: &mut bus };

        let _ = view.fetch_opcode_byte(0);
        assert_eq!(view.drain_wait_cycles(), 1);
        assert_eq!(view.drain_wait_cycles(), 0);

        let _ = view.read_byte(0);
        view.write_byte(0, 0);
        let _ = view.io_read_byte(0x98);
        view.io_write_byte(0x98, 0);
        assert_eq!(view.drain_wait_cycles(), 0);
    }
}
