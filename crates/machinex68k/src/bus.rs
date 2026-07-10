//! X68000 24-bit address decoder, memory, and motherboard registers.

mod adpcm;
mod dmac;
mod fdc;
mod gvram;
mod midi;
mod ppi;
mod printer;
mod read;
mod scc;
mod sound;
mod storage;
mod system_port;
mod video;
mod write;

use common::{
    Bus, CpuMode, HostDateTimeProvider, JoystickState, M68000AccessSize, M68000BusAccess,
    M68000BusError, M68000CycleKind, M68000FunctionCode, NoTracing, Tracing,
};
use device::{
    crtc_x68k::CrtcX68k,
    fdd_x68k::FddX68k,
    floppy::MountedFloppy,
    hd63450_dmac::Hd63450Dmac,
    i8255::I8255,
    keyboard_x68k::{KEYBOARD_X68K_TICKS_PER_SECOND, KeyboardX68k},
    mc68901_mfp::{MC68901_CLOCK_HZ, Mc68901Mfp},
    msm6258::Msm6258,
    opn_fm::{OpnFm, Ym2151},
    rp5c15_rtc::{RP5C15_CLOCK_HZ, Rp5c15Rtc},
    sasi::X68kSasiHdc,
    scsi::Mb89352Spc,
    sprite_x68k::SpriteX68k,
    upd72065_fdc::Upd72065Fdc,
    video_controller_x68k::VideoControllerX68k,
    ym3802::{YM3802_CLKM_HZ, Ym3802},
    z8530::Z8530,
};
use software_renderer::x68k::X68kRenderer;

use crate::{
    InterruptSource, IocSource, LoadedRoms, X68kModel,
    bus::{
        dmac::DMAC_CLOCK_HZ,
        scc::{MouseState, SCC_CLOCK_HZ},
        sound::OPM_CLOCK_HZ,
    },
    clock::{cycle_to_tick, tick_to_cycle},
    interrupt::InterruptRouter,
    scheduler::{EventX68k, X68kScheduler},
    sram::Sram,
};

/// Mask applied to every MC68000 address.
const ADDRESS_MASK: u32 = 0x00FF_FFFF;
/// One mebibyte in bytes.
const MEBIBYTE: usize = 1 << 20;
/// Maximum main RAM size supported by the X68000 memory map.
pub const X68K_MAX_MAIN_RAM_SIZE: usize = 12 * MEBIBYTE;
/// Default main RAM size used by Neetan's X68000 machines.
pub const X68K_DEFAULT_MAIN_RAM_SIZE: usize = X68K_MAX_MAIN_RAM_SIZE;
/// Number of 16-bit graphics VRAM words backing all page windows.
const GVRAM_WORDS: usize = 0x4_0000;
/// Size of the text VRAM address window.
const TVRAM_SIZE: usize = 0x08_0000;
/// Size of the character-generator ROM.
const CGROM_SIZE: usize = 0x0C_0000;
/// Size of the mapped IPL ROM.
const IPL_SIZE: usize = 0x02_0000;
/// Size of the SUPER/XVI internal-SCSI ROM window.
const SCSI_WINDOW_SIZE: usize = 0x02_0000;

/// Decoded X68000 address region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X68kRegion {
    /// Main RAM.
    MainRam,
    /// Graphics VRAM.
    GraphicVram,
    /// Text VRAM.
    TextVram,
    /// X68000 CRTC registers and operation port.
    Crtc,
    /// Graphics and text palette RAM.
    Palette,
    /// X68000 video-controller registers.
    VideoController,
    /// Motorola MC68901 MFP.
    Mfp,
    /// Ricoh RP5C15 RTC.
    Rtc,
    /// Standard main-RAM supervisor-area register.
    StandardSupervisorArea,
    /// Mirrored system-port register block.
    SystemPort,
    /// Interrupt I/O controller registers.
    Ioc,
    /// Enhanced main-RAM supervisor-area registers.
    EnhancedSupervisorArea,
    /// HD63450 DMA controller registers.
    Dmac,
    /// uPD72065 FDC and drive-control registers.
    Fdc,
    /// CYNTHIA sprite controller and pattern RAM.
    Sprite,
    /// i8255 PPI for joysticks and ADPCM control.
    Ppi,
    /// Z8530 serial communications controller.
    Scc,
    /// Parallel printer interface.
    Printer,
    /// SASI or internal SCSI disk controller window.
    StorageController,
    /// YM2151 OPM register and status ports.
    Opm,
    /// MSM6258 ADPCM command, status, and data ports.
    Adpcm,
    /// CZ-6BM1 MIDI card register window.
    Midi,
    /// Reserved built-in I/O space that raises bus errors.
    BuiltinDevice,
    /// User expansion I/O space.
    UserIo,
    /// Battery-backed SRAM.
    Sram,
    /// Unmapped address hole.
    Unmapped,
    /// Character-generator ROM.
    Cgrom,
    /// Internal SCSI boot-ROM window.
    InternalScsiRom,
    /// IPL ROM.
    IplRom,
}

/// Number of CPU main-RAM accesses that share one DRAM refresh wait cycle.
const CPU_RAM_ACCESSES_PER_REFRESH_CYCLE: u8 = 8;

/// Returns the CPU wait penalty of one bus access to the region, in whole
/// CPU cycles. Main RAM carries only the shared DRAM refresh cycle counted
/// separately; regions that never complete an access and the expansion MIDI
/// window carry no penalty.
const fn cpu_access_wait_cycles(region: X68kRegion) -> u64 {
    match region {
        X68kRegion::MainRam => 0,
        X68kRegion::VideoController
        | X68kRegion::Crtc
        | X68kRegion::Printer
        | X68kRegion::SystemPort
        | X68kRegion::Sprite
        | X68kRegion::Sram
        | X68kRegion::Cgrom
        | X68kRegion::InternalScsiRom
        | X68kRegion::IplRom
        | X68kRegion::StandardSupervisorArea
        | X68kRegion::EnhancedSupervisorArea
        | X68kRegion::GraphicVram => 1,
        X68kRegion::Rtc
        | X68kRegion::Opm
        | X68kRegion::Adpcm
        | X68kRegion::Fdc
        | X68kRegion::StorageController
        | X68kRegion::Ppi
        | X68kRegion::Ioc
        | X68kRegion::TextVram => 2,
        X68kRegion::Palette => 3,
        X68kRegion::Mfp => 4,
        X68kRegion::Scc => 6,
        X68kRegion::Dmac => 15,
        X68kRegion::Midi
        | X68kRegion::BuiltinDevice
        | X68kRegion::UserIo
        | X68kRegion::Unmapped => 0,
    }
}

/// X68000 system bus and motherboard state.
pub struct X68kBus<T: Tracing = NoTracing> {
    model: X68kModel,
    cpu_mode: CpuMode,
    ram: Box<[u8]>,
    graphic_vram: Box<[u16]>,
    text_vram: Box<[u8]>,
    cgrom: Vec<u8>,
    ipl: Vec<u8>,
    internal_scsi: Option<Box<[u8]>>,
    sram: Sram,
    sram_write_enabled: bool,
    standard_supervisor_area: u8,
    enhanced_supervisor_area: [u8; 5],
    contrast: u8,
    monitor_control: u8,
    color_latch: u8,
    key_control: u8,
    shutdown_sequence: u8,
    shutdown_requested: bool,
    interrupts: InterruptRouter,
    crtc: CrtcX68k,
    video_controller: VideoControllerX68k,
    sprite: SpriteX68k,
    ppi: I8255,
    joystick_ports: [JoystickState; 2],
    scc: Z8530,
    mouse: MouseState,
    printer_data: u8,
    printer_strobe: u8,
    mfp: Mc68901Mfp,
    rtc: Rp5c15Rtc,
    keyboard: KeyboardX68k,
    dmac: Hd63450Dmac,
    fdc: Upd72065Fdc,
    fdd: FddX68k,
    hdc: X68kSasiHdc,
    spc: Mb89352Spc,
    spc_irq_line: bool,
    floppy_drives: [Option<MountedFloppy>; 4],
    opm: OpnFm<Ym2151>,
    adpcm: Msm6258,
    midi_card: Option<Ym3802>,
    #[cfg(feature = "mt32")]
    mt32: Option<device::mt32::Mt32>,
    #[cfg(feature = "sc55")]
    sc55: Option<device::sc55::Sc55>,
    fdc_forced_ready: bool,
    adpcm_cycle_remainder: u64,
    renderer: X68kRenderer,
    scheduler: X68kScheduler,
    sample_rate: u32,
    cpu_clock_hz: u64,
    crtc_last_cycle: u64,
    crtc_remainder: u64,
    wait_cycles: i64,
    cpu_refresh_access_count: u8,
    dmac_stall_remainder: u64,
    dmac_wait_clocks: u64,
    dmac_refresh_access_count: u8,
    host_date_time_provider: HostDateTimeProvider,
    current_cycle: u64,
    tracer: T,
}

impl<T: Tracing> X68kBus<T> {
    /// Builds a bus from a validated model-specific ROM set.
    pub fn new(
        model: X68kModel,
        cpu_mode: CpuMode,
        roms: LoadedRoms,
        sample_rate: u32,
    ) -> Result<Self, String>
    where
        T: Default,
    {
        Self::with_main_ram_size(
            model,
            cpu_mode,
            roms,
            sample_rate,
            X68K_DEFAULT_MAIN_RAM_SIZE,
        )
    }

    /// Builds a bus with an explicit main-RAM size.
    pub fn with_main_ram_size(
        model: X68kModel,
        cpu_mode: CpuMode,
        roms: LoadedRoms,
        sample_rate: u32,
        main_ram_size: usize,
    ) -> Result<Self, String>
    where
        T: Default,
    {
        validate_main_ram_size(main_ram_size)?;
        if roms.model != model {
            return Err(format!(
                "ROM set for {} cannot initialize {model}",
                roms.model
            ));
        }
        if roms.cgrom.len() != CGROM_SIZE || roms.ipl.len() != IPL_SIZE {
            return Err("invalid X68000 CGROM or IPL size".into());
        }
        if model.has_internal_scsi() != roms.internal_scsi.is_some() {
            return Err(format!("incorrect internal SCSI ROM presence for {model}"));
        }
        let internal_scsi = if let Some(scsi) = roms.internal_scsi {
            if scsi.len() != 0x2000 {
                return Err("invalid X68000 internal SCSI ROM size".into());
            }
            let mut window = vec![0xFF; SCSI_WINDOW_SIZE].into_boxed_slice();
            window[..scsi.len()].copy_from_slice(&scsi);
            Some(window)
        } else {
            None
        };
        let cpu_clock_hz = u64::from(model.cpu_clock_hz(cpu_mode));
        let mut bus = Self {
            model,
            cpu_mode,
            ram: vec![0; main_ram_size].into_boxed_slice(),
            graphic_vram: vec![0; GVRAM_WORDS].into_boxed_slice(),
            text_vram: vec![0; TVRAM_SIZE].into_boxed_slice(),
            cgrom: roms.cgrom,
            ipl: roms.ipl,
            internal_scsi,
            sram: Sram::new(model, main_ram_size),
            sram_write_enabled: false,
            standard_supervisor_area: 0,
            enhanced_supervisor_area: [0; 5],
            contrast: 0,
            monitor_control: 0,
            color_latch: 0,
            key_control: 0x08,
            shutdown_sequence: 0,
            shutdown_requested: false,
            interrupts: InterruptRouter::default(),
            crtc: CrtcX68k::new(),
            video_controller: VideoControllerX68k::new(),
            sprite: SpriteX68k::new(),
            ppi: x68k_ppi(),
            joystick_ports: [JoystickState::default(); 2],
            scc: Z8530::new(),
            mouse: MouseState::default(),
            printer_data: 0,
            printer_strobe: 1,
            mfp: Mc68901Mfp::new(),
            rtc: Rp5c15Rtc::new(),
            keyboard: KeyboardX68k::new(),
            dmac: Hd63450Dmac::new(),
            fdc: Upd72065Fdc::new(),
            fdd: FddX68k::new(),
            hdc: X68kSasiHdc::new(cpu_clock_hz),
            spc: Mb89352Spc::new(cpu_clock_hz),
            spc_irq_line: false,
            floppy_drives: [None, None, None, None],
            opm: OpnFm::new(model.cpu_clock_hz(cpu_mode), sample_rate, OPM_CLOCK_HZ),
            adpcm: Msm6258::new(sample_rate),
            midi_card: None,
            #[cfg(feature = "mt32")]
            mt32: None,
            #[cfg(feature = "sc55")]
            sc55: None,
            fdc_forced_ready: false,
            adpcm_cycle_remainder: 0,
            renderer: X68kRenderer::new(),
            scheduler: X68kScheduler::new(),
            sample_rate,
            cpu_clock_hz,
            crtc_last_cycle: 0,
            crtc_remainder: 0,
            wait_cycles: 0,
            cpu_refresh_access_count: 0,
            dmac_stall_remainder: 0,
            dmac_wait_clocks: 0,
            dmac_refresh_access_count: 0,
            host_date_time_provider: common::default_host_date_time,
            current_cycle: 0,
            tracer: T::default(),
        };
        bus.initialize_device_pins();
        bus.schedule_events();
        Ok(bus)
    }

    /// Returns the selected model.
    pub const fn model(&self) -> X68kModel {
        self.model
    }

    /// Returns the installed main-RAM size in bytes.
    pub fn main_ram_size(&self) -> usize {
        self.ram.len()
    }

    /// A shared reference to the bus-activity tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// A mutable reference to the bus-activity tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Classifies a 24-bit address without performing an access.
    pub const fn decode_region(address: u32) -> X68kRegion {
        let address = address & ADDRESS_MASK;
        match address {
            0x000000..=0xBFFFFF => X68kRegion::MainRam,
            0xC00000..=0xDFFFFF => X68kRegion::GraphicVram,
            0xE00000..=0xE7FFFF => X68kRegion::TextVram,
            0xE80000..=0xE81FFF => X68kRegion::Crtc,
            0xE82000..=0xE823FF => X68kRegion::Palette,
            0xE82400..=0xE83FFF => X68kRegion::VideoController,
            0xE86000..=0xE87FFF => X68kRegion::StandardSupervisorArea,
            0xE88000..=0xE89FFF => X68kRegion::Mfp,
            0xE8A000..=0xE8BFFF => X68kRegion::Rtc,
            0xE8C000..=0xE8DFFF => X68kRegion::Printer,
            0xE84000..=0xE85FFF => X68kRegion::Dmac,
            0xE8E000..=0xE8FFFF => X68kRegion::SystemPort,
            0xE90000..=0xE91FFF => X68kRegion::Opm,
            0xE92000..=0xE93FFF => X68kRegion::Adpcm,
            0xE94000..=0xE95FFF => X68kRegion::Fdc,
            0xE96000..=0xE97FFF => X68kRegion::StorageController,
            0xE9C000..=0xE9C003 => X68kRegion::Ioc,
            0xE9A000..=0xE9BFFF => X68kRegion::Ppi,
            0xE98000..=0xE99FFF => X68kRegion::Scc,
            0xEAFA00..=0xEAFA0F => X68kRegion::Midi,
            0xEAFF80..=0xEAFF89 => X68kRegion::EnhancedSupervisorArea,
            0xEB0000..=0xEBFFFF => X68kRegion::Sprite,
            0xE9C004..=0xEAFFFF => X68kRegion::BuiltinDevice,
            0xEC0000..=0xECFFFF => X68kRegion::UserIo,
            0xED0000..=0xED3FFF => X68kRegion::Sram,
            0xED4000..=0xEFFFFF => X68kRegion::Unmapped,
            0xF00000..=0xFBFFFF => X68kRegion::Cgrom,
            0xFC0000..=0xFDFFFF => X68kRegion::InternalScsiRom,
            0xFE0000..=0xFFFFFF => X68kRegion::IplRom,
            _ => X68kRegion::Unmapped,
        }
    }

    /// Returns a main-RAM byte for diagnostics and tests.
    pub fn ram_byte(&self, address: u32) -> Option<u8> {
        self.ram.get(address as usize).copied()
    }

    /// Returns the current in-memory SRAM image.
    pub fn sram_data(&self) -> &[u8] {
        self.sram.data()
    }

    /// Returns the loaded character-generator ROM.
    pub fn cgrom_data(&self) -> &[u8] {
        &self.cgrom
    }

    /// Returns text VRAM for diagnostics and reference-ROM tests.
    pub fn text_vram_data(&self) -> &[u8] {
        &self.text_vram
    }

    /// Returns the packed graphics VRAM words for diagnostics and tests.
    pub fn graphic_vram_data(&self) -> &[u16] {
        &self.graphic_vram
    }

    /// Returns the completed video frame.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.renderer.framebuffer()
    }

    /// Returns the completed video-frame dimensions.
    pub fn display_dimensions(&self) -> (u32, u32) {
        self.renderer.dimensions()
    }

    /// Installs the local-calendar provider used for the first RTC access.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
    }

    /// Updates a physical X68000 key from a make or break code.
    pub fn push_keyboard_scancode(&mut self, value: u8) {
        self.synchronize_devices();
        let tick = cycle_to_tick(
            self.current_cycle,
            KEYBOARD_X68K_TICKS_PER_SECOND,
            self.cpu_clock_hz,
        );
        self.keyboard
            .set_key_state(value & 0x7F, value & 0x80 == 0, tick);
        self.shuttle_keyboard_serial();
        self.schedule_events();
    }

    /// Returns the next scheduled motherboard event cycle.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Processes all peripheral events due at the current cycle.
    pub fn process_due_events(&mut self) {
        let due = self.scheduler.pop_due_events(self.current_cycle);
        if due.is_empty() {
            return;
        }
        self.synchronize_devices();
        for event in due.iter() {
            self.tracer.trace_event(event.fire_cycle, event.kind as u8);
            match event.kind {
                EventX68k::Crtc
                | EventX68k::Mfp
                | EventX68k::Rtc
                | EventX68k::Keyboard
                | EventX68k::Dmac
                | EventX68k::Midi
                | EventX68k::SccMouse => {}
                EventX68k::Fdc => self.on_fdc_drq(),
                EventX68k::FdcInterrupt => self.on_fdc_seek_complete(),
                EventX68k::Hdc => self.on_storage_hdc_due(),
                EventX68k::Spc => self.on_storage_spc_due(),
                EventX68k::OpmTimerA => self.on_opm_timer_expired(0, event.fire_cycle),
                EventX68k::OpmTimerB => self.on_opm_timer_expired(1, event.fire_cycle),
                EventX68k::Adpcm => self.on_adpcm_byte_tick(event.fire_cycle),
            }
        }
        self.pump_dmac();
    }

    /// Returns whether the shutdown register sequence completed.
    pub const fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Asserts a vectored motherboard interrupt input.
    pub fn assert_interrupt(&mut self, source: InterruptSource, vector: u8) {
        self.interrupts.assert(source, vector);
    }

    /// Clears a motherboard interrupt input.
    pub fn clear_interrupt(&mut self, source: InterruptSource) {
        self.interrupts.clear(source);
    }

    /// Latches an IOC interrupt edge.
    pub fn signal_ioc_interrupt(&mut self, source: IocSource) {
        self.interrupts.ioc.signal(source);
    }

    /// Clears a latched IOC interrupt request.
    pub fn clear_ioc_interrupt(&mut self, source: IocSource) {
        self.interrupts.ioc.clear(source);
    }

    /// Resets motherboard state.
    fn reset_devices(&mut self) {
        self.sram_write_enabled = false;
        self.standard_supervisor_area = 0;
        self.enhanced_supervisor_area = [0; 5];
        self.contrast = 0;
        self.monitor_control = 0;
        self.color_latch = 0;
        self.key_control = 0x08;
        self.shutdown_sequence = 0;
        self.shutdown_requested = false;
        self.interrupts.reset();
        self.crtc.reset();
        self.video_controller.reset();
        self.sprite.reset();
        self.ppi = x68k_ppi();
        self.joystick_ports = [JoystickState::default(); 2];
        self.scc.reset();
        self.mouse = MouseState::default();
        self.printer_data = 0;
        self.printer_strobe = 1;
        self.mfp.reset();
        self.rtc.reset();
        self.keyboard.reset();
        self.dmac.reset();
        self.fdc.write_auxiliary_command(0x36);
        self.fdd.reset();
        self.hdc.write_reset(0);
        self.spc.hard_reset();
        self.spc_irq_line = false;
        self.opm = OpnFm::new(self.cpu_clock_hz as u32, self.sample_rate, OPM_CLOCK_HZ);
        self.adpcm.reset();
        if let Some(chip) = self.midi_card.as_mut() {
            chip.reset();
        }
        self.fdc_forced_ready = false;
        self.adpcm_cycle_remainder = 0;
        self.sync_fdc_ready_lines();
        self.crtc_last_cycle = self.current_cycle;
        self.crtc_remainder = 0;
        self.cpu_refresh_access_count = 0;
        self.dmac_stall_remainder = 0;
        self.dmac_wait_clocks = 0;
        self.dmac_refresh_access_count = 0;
        self.scheduler = X68kScheduler::new();
        self.initialize_device_pins();
        self.schedule_events();
    }

    /// Charges the region wait of one successful CPU bus access, counting
    /// the DRAM refresh cycle shared by every eight main-RAM accesses.
    fn charge_cpu_access_wait(&mut self, region: X68kRegion) {
        self.wait_cycles += cpu_access_wait_cycles(region) as i64;
        if region == X68kRegion::MainRam {
            self.cpu_refresh_access_count += 1;
            if self.cpu_refresh_access_count == CPU_RAM_ACCESSES_PER_REFRESH_CYCLE {
                self.cpu_refresh_access_count = 0;
                self.wait_cycles += 1;
            }
        }
    }

    /// Validates and decodes one bus transaction.
    fn check_access(&self, access: M68000BusAccess) -> Result<(u32, X68kRegion), M68000BusError> {
        let address = access.address & ADDRESS_MASK;
        if access.size == M68000AccessSize::Word && address & 1 != 0 {
            return Err(M68000BusError);
        }
        let end = address + u32::from(access.size == M68000AccessSize::Word);
        let region = Self::decode_region(address);
        if Self::decode_region(end) != region {
            return Err(M68000BusError);
        }
        if is_user(access.function_code) && self.is_supervisor_only(address, end, region) {
            return Err(M68000BusError);
        }
        if region == X68kRegion::MainRam && end as usize >= self.ram.len() {
            return Err(M68000BusError);
        }
        match region {
            X68kRegion::BuiltinDevice | X68kRegion::UserIo | X68kRegion::Unmapped => {
                Err(M68000BusError)
            }
            X68kRegion::Sprite if self.sprite_window_blocked(address) => Err(M68000BusError),
            X68kRegion::Midi if self.midi_card.is_none() => Err(M68000BusError),
            X68kRegion::InternalScsiRom if self.internal_scsi.is_none() => Err(M68000BusError),
            _ => Ok((address, region)),
        }
    }

    /// Checks whether the sprite window rejects an access under CRTC R20.
    ///
    /// The screen-timing registers and the hole above them stay reachable
    /// even while the sprite RAM itself is inaccessible.
    fn sprite_window_blocked(&self, address: u32) -> bool {
        if self.crtc.sprite_area_accessible() {
            return false;
        }
        !matches!(address, 0xEB0800..=0xEB083F | 0xEB4000..=0xEB7FFF)
    }

    /// Checks whether a region requires supervisor access.
    fn is_supervisor_only(&self, start: u32, end: u32, region: X68kRegion) -> bool {
        match region {
            X68kRegion::MainRam => {
                self.ram_address_protected(start) || self.ram_address_protected(end)
            }
            X68kRegion::GraphicVram
            | X68kRegion::TextVram
            | X68kRegion::Crtc
            | X68kRegion::Palette
            | X68kRegion::VideoController
            | X68kRegion::StandardSupervisorArea
            | X68kRegion::Mfp
            | X68kRegion::Rtc
            | X68kRegion::SystemPort
            | X68kRegion::Ioc
            | X68kRegion::EnhancedSupervisorArea
            | X68kRegion::Dmac
            | X68kRegion::Fdc
            | X68kRegion::Sprite
            | X68kRegion::Ppi
            | X68kRegion::Scc
            | X68kRegion::Printer
            | X68kRegion::StorageController
            | X68kRegion::Opm
            | X68kRegion::Adpcm
            | X68kRegion::Midi
            | X68kRegion::BuiltinDevice
            | X68kRegion::Sram
            | X68kRegion::Cgrom
            | X68kRegion::InternalScsiRom
            | X68kRegion::IplRom => true,
            X68kRegion::UserIo | X68kRegion::Unmapped => false,
        }
    }

    /// Checks main-RAM supervisor protection.
    fn ram_address_protected(&self, address: u32) -> bool {
        let standard_limit = (u32::from(self.standard_supervisor_area) + 1) * 0x2000;
        if address < standard_limit {
            return true;
        }
        if !(0x200000..0xC00000).contains(&address) {
            return false;
        }
        let relative = address - 0x200000;
        let register = (relative / 0x200000) as usize;
        let bit = ((relative % 0x200000) / 0x40000) as u8;
        self.enhanced_supervisor_area[register] & (1 << bit) != 0
    }

    /// Advances all phase-2 devices to the current CPU cycle.
    fn synchronize_devices(&mut self) {
        self.catch_up_video();
        let elapsed_cycles = self.current_cycle.saturating_sub(self.crtc_last_cycle);
        if elapsed_cycles != 0 {
            let oscillator_hz = u64::from(self.crtc.oscillator_hz());
            let total = u128::from(elapsed_cycles) * u128::from(oscillator_hz)
                + u128::from(self.crtc_remainder);
            let ticks = (total / u128::from(self.cpu_clock_hz)) as u64;
            self.crtc_remainder = (total % u128::from(self.cpu_clock_hz)) as u64;
            self.crtc_last_cycle = self.current_cycle;
            let transitions = self.crtc.advance_oscillator_ticks(ticks);
            for _ in 0..transitions.raster_copies {
                self.execute_raster_copy();
            }
            if transitions.frame_started {
                self.publish_video_frame();
            }
            if transitions.vertical_display_started {
                if self.crtc.high_speed_clear_active() {
                    self.crtc.complete_high_speed_clear();
                }
                if self.crtc.high_speed_clear_requested() {
                    self.crtc.begin_high_speed_clear();
                    self.execute_high_speed_clear();
                }
            }
        }
        let mfp_tick = cycle_to_tick(self.current_cycle, MC68901_CLOCK_HZ, self.cpu_clock_hz);
        let rtc_tick = cycle_to_tick(self.current_cycle, RP5C15_CLOCK_HZ, self.cpu_clock_hz);
        let keyboard_tick = cycle_to_tick(
            self.current_cycle,
            KEYBOARD_X68K_TICKS_PER_SECOND,
            self.cpu_clock_hz,
        );
        self.mfp.advance_to(mfp_tick);
        self.rtc.advance_to(rtc_tick);
        self.keyboard.advance_to(keyboard_tick);
        let scc_tick = cycle_to_tick(self.current_cycle, SCC_CLOCK_HZ, self.cpu_clock_hz);
        self.scc.advance_to(scc_tick);
        if let Some(chip) = self.midi_card.as_mut() {
            let midi_tick = cycle_to_tick(self.current_cycle, YM3802_CLKM_HZ, self.cpu_clock_hz);
            chip.advance_to(midi_tick);
        }
        self.update_device_pins();
        self.shuttle_keyboard_serial();
        self.catch_up_video();
        self.schedule_events();
    }

    /// Transfers completed serial bytes between the MFP and keyboard.
    fn shuttle_keyboard_serial(&mut self) {
        let keyboard_tick = cycle_to_tick(
            self.current_cycle,
            KEYBOARD_X68K_TICKS_PER_SECOND,
            self.cpu_clock_hz,
        );
        if let Some(command) = self.mfp.take_transmitted_byte() {
            self.keyboard.write_command(command, keyboard_tick);
        }
        if self.mfp.receiver_idle()
            && let Some(value) = self.keyboard.take_output_byte()
        {
            let mfp_tick = cycle_to_tick(self.current_cycle, MC68901_CLOCK_HZ, self.cpu_clock_hz);
            let _ = self.mfp.begin_receive_byte(value, mfp_tick);
        }
    }

    /// Seeds the RTC from the host calendar exactly once.
    fn seed_rtc(&mut self) {
        if self.rtc.seeded() {
            return;
        }
        let mut calendar = (self.host_date_time_provider)().to_bcd_bytes();
        let year = bcd_to_binary(calendar[0]);
        calendar[0] = binary_to_bcd(if year >= 80 { year - 80 } else { year + 20 });
        self.rtc.seed_from_calendar_bcd(calendar);
    }

    /// Establishes fixed motherboard input levels after reset.
    fn initialize_device_pins(&mut self) {
        let tick = cycle_to_tick(self.current_cycle, MC68901_CLOCK_HZ, self.cpu_clock_hz);
        for (bit, level) in [(1, true), (2, false), (5, true)] {
            self.mfp.set_gpip_input(bit, level, tick);
        }
        self.update_device_pins();
    }

    /// Propagates CRTC, RTC, and OPM output pins into the MFP.
    fn update_device_pins(&mut self) {
        let tick = cycle_to_tick(self.current_cycle, MC68901_CLOCK_HZ, self.cpu_clock_hz);
        let signals = self.crtc.signals();
        self.mfp.set_gpip_input(0, self.rtc.alarm_level(), tick);
        self.mfp.set_gpip_input(3, !self.opm.irq_asserted(), tick);
        self.mfp.set_gpip_input(4, signals.vertical_display, tick);
        self.mfp.set_gpip_input(6, signals.raster_interrupt, tick);
        self.mfp.set_gpip_input(7, signals.horizontal_sync, tick);
    }

    /// Rebuilds the machine event deadlines from each device.
    fn schedule_events(&mut self) {
        let schedule_absolute = |scheduler: &mut X68kScheduler,
                                 kind,
                                 deadline,
                                 frequency,
                                 cpu_clock_hz,
                                 current_cycle| {
            if let Some(deadline) = deadline {
                let cycle = tick_to_cycle(deadline, frequency, cpu_clock_hz);
                scheduler.schedule(kind, cycle.max(current_cycle + 1));
            } else {
                scheduler.cancel(kind);
            }
        };
        if let Some(ticks) = self.crtc.ticks_until_transition() {
            let numerator = u128::from(ticks) * u128::from(self.cpu_clock_hz);
            let cycles = numerator.div_ceil(u128::from(self.crtc.oscillator_hz())) as u64;
            self.scheduler
                .schedule(EventX68k::Crtc, self.current_cycle + cycles.max(1));
        } else {
            self.scheduler.cancel(EventX68k::Crtc);
        }
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::Mfp,
            self.mfp.next_event_tick(),
            MC68901_CLOCK_HZ,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::Rtc,
            self.rtc.next_event_tick(),
            RP5C15_CLOCK_HZ,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::Keyboard,
            self.keyboard.next_event_tick(),
            KEYBOARD_X68K_TICKS_PER_SECOND,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::Dmac,
            self.dmac.next_work_clock(),
            DMAC_CLOCK_HZ,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::SccMouse,
            self.scc.next_event_tick(),
            SCC_CLOCK_HZ,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        schedule_absolute(
            &mut self.scheduler,
            EventX68k::Midi,
            self.midi_card.as_ref().and_then(Ym3802::next_event_tick),
            YM3802_CLKM_HZ,
            self.cpu_clock_hz,
            self.current_cycle,
        );
        if let Some(cycle) = self.hdc.next_event_cycle() {
            self.scheduler
                .schedule(EventX68k::Hdc, cycle.max(self.current_cycle + 1));
        } else {
            self.scheduler.cancel(EventX68k::Hdc);
        }
        if let Some(cycle) = self.spc.next_event_cycle() {
            self.scheduler
                .schedule(EventX68k::Spc, cycle.max(self.current_cycle + 1));
        } else {
            self.scheduler.cancel(EventX68k::Spc);
        }
    }
}

/// Checks for a user-mode function code.
fn is_user(function_code: M68000FunctionCode) -> bool {
    matches!(
        function_code,
        M68000FunctionCode::UserData | M68000FunctionCode::UserProgram
    )
}

/// Checks for any register region.
fn is_register_region(region: X68kRegion) -> bool {
    matches!(
        region,
        X68kRegion::Crtc
            | X68kRegion::Palette
            | X68kRegion::VideoController
            | X68kRegion::Sprite
            | X68kRegion::StandardSupervisorArea
            | X68kRegion::Mfp
            | X68kRegion::Rtc
            | X68kRegion::SystemPort
            | X68kRegion::Ioc
            | X68kRegion::Ppi
            | X68kRegion::Scc
            | X68kRegion::Printer
            | X68kRegion::StorageController
            | X68kRegion::Opm
            | X68kRegion::Adpcm
            | X68kRegion::Midi
            | X68kRegion::EnhancedSupervisorArea
    )
}

/// Checks for a native 16-bit register region.
fn is_word_device_region(region: X68kRegion) -> bool {
    matches!(
        region,
        X68kRegion::Crtc | X68kRegion::Palette | X68kRegion::VideoController | X68kRegion::Sprite
    )
}

/// Checks for a register connected to the lower byte lane.
fn is_lower_lane_register_region(region: X68kRegion) -> bool {
    matches!(
        region,
        X68kRegion::StandardSupervisorArea
            | X68kRegion::Mfp
            | X68kRegion::Rtc
            | X68kRegion::SystemPort
            | X68kRegion::Ioc
            | X68kRegion::Ppi
            | X68kRegion::Scc
            | X68kRegion::Printer
            | X68kRegion::StorageController
            | X68kRegion::Opm
            | X68kRegion::Adpcm
            | X68kRegion::Midi
            | X68kRegion::EnhancedSupervisorArea
    )
}

/// Builds the motherboard PPI with both joystick ports idle.
fn x68k_ppi() -> I8255 {
    let mut ppi = I8255::new();
    ppi.set_port_a(0xFF);
    ppi.set_port_b(0xFF);
    ppi
}

/// Maps an enhanced-area register address to its index.
fn enhanced_register_index(address: u32) -> Option<usize> {
    match address {
        0xEAFF81 => Some(0),
        0xEAFF83 => Some(1),
        0xEAFF85 => Some(2),
        0xEAFF87 => Some(3),
        0xEAFF89 => Some(4),
        _ => None,
    }
}

fn bcd_to_binary(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0F)
}

fn binary_to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

impl<T: Tracing> Bus for X68kBus<T> {
    /// Reads through the legacy supervisor bridge.
    fn read_byte(&mut self, address: u32) -> u8 {
        self.read_checked(M68000BusAccess {
            address,
            size: M68000AccessSize::Byte,
            function_code: M68000FunctionCode::SupervisorData,
            cycle_kind: M68000CycleKind::Normal,
        })
        .unwrap_or(0xFF) as u8
    }

    /// Writes through the legacy supervisor bridge.
    fn write_byte(&mut self, address: u32, value: u8) {
        let _ = self.write_checked(
            M68000BusAccess {
                address,
                size: M68000AccessSize::Byte,
                function_code: M68000FunctionCode::SupervisorData,
                cycle_kind: M68000CycleKind::Normal,
            },
            u16::from(value),
        );
    }

    /// Returns open bus for unused port I/O.
    fn io_read_byte(&mut self, _port: u16) -> u8 {
        0xFF
    }

    /// Ignores unused port I/O writes.
    fn io_write_byte(&mut self, _port: u16, _value: u8) {}

    /// Reports no x86-style IRQ.
    fn has_irq(&self) -> bool {
        false
    }

    /// Returns no x86-style IRQ vector.
    fn acknowledge_irq(&mut self) -> u8 {
        0
    }

    /// Reports no x86-style NMI.
    fn has_nmi(&self) -> bool {
        false
    }

    /// Acknowledges no x86-style NMI.
    fn acknowledge_nmi(&mut self) {}

    /// Returns the highest MC68000 interrupt level.
    fn m68000_interrupt_level(&self) -> u8 {
        self.interrupts
            .level()
            .max(if self.mfp.irq_asserted() { 6 } else { 0 })
            .max(if self.scc.irq_asserted() { 5 } else { 0 })
            .max(
                if self.midi_card.as_ref().is_some_and(Ym3802::irq_asserted) {
                    4
                } else {
                    0
                },
            )
            .max(if self.dmac.irq_asserted() { 3 } else { 0 })
    }

    /// Acknowledges an MC68000 interrupt.
    fn m68000_acknowledge_interrupt(&mut self, level: u8) -> u8 {
        if level == 6
            && self.mfp.irq_asserted()
            && let Some(vector) = self.mfp.acknowledge_interrupt()
        {
            self.tracer.trace_irq_acknowledge(level, vector);
            return vector;
        }
        if level == 5
            && self.scc.irq_asserted()
            && let Some(vector) = self.scc.acknowledge_interrupt()
        {
            self.tracer.trace_irq_acknowledge(level, vector);
            return vector;
        }
        if level == 4
            && let Some(chip) = self.midi_card.as_mut()
            && chip.irq_asserted()
            && let Some(vector) = chip.acknowledge_interrupt()
        {
            self.tracer.trace_irq_acknowledge(level, vector);
            return vector;
        }
        if level == 3
            && self.dmac.irq_asserted()
            && let Some(vector) = self.dmac.acknowledge_interrupt()
        {
            self.tracer.trace_irq_acknowledge(level, vector);
            return vector;
        }
        let vector = self.interrupts.acknowledge(level);
        self.tracer.trace_irq_acknowledge(level, vector);
        vector
    }

    /// Resets motherboard state on RESET assertion.
    fn m68000_reset_line(&mut self, asserted: bool) {
        if asserted {
            self.reset_devices();
        }
    }

    /// Performs a typed MC68000 read.
    fn m68000_read(&mut self, access: M68000BusAccess) -> Result<u16, M68000BusError> {
        if access.function_code == M68000FunctionCode::CpuSpace {
            let level = ((access.address >> 1) & 7) as u8;
            return Ok(u16::from(self.m68000_acknowledge_interrupt(level)));
        }
        let value = self.read_checked(access)?;
        self.charge_cpu_access_wait(Self::decode_region(access.address));
        match access.size {
            M68000AccessSize::Byte => self.tracer.trace_mem_read(access.address, value as u8),
            M68000AccessSize::Word => self.tracer.trace_mem_read_word(access.address, value),
        }
        Ok(value)
    }

    /// Performs a typed MC68000 write.
    fn m68000_write(&mut self, access: M68000BusAccess, value: u16) -> Result<(), M68000BusError> {
        match access.size {
            M68000AccessSize::Byte => self.tracer.trace_mem_write(access.address, value as u8),
            M68000AccessSize::Word => self.tracer.trace_mem_write_word(access.address, value),
        }
        self.write_checked(access, value)?;
        self.charge_cpu_access_wait(Self::decode_region(access.address));
        Ok(())
    }

    /// Returns the current CPU cycle.
    fn current_cycle(&self) -> u64 {
        self.current_cycle
    }

    /// Sets the current CPU cycle.
    fn set_current_cycle(&mut self, cycle: u64) {
        self.current_cycle = cycle;
        self.tracer.set_cycle(cycle);
    }

    /// Drains CPU cycles stolen by DMAC bus mastery.
    fn drain_wait_cycles(&mut self) -> i64 {
        std::mem::take(&mut self.wait_cycles)
    }
}

/// Validates an X68000 main-RAM size in bytes.
fn validate_main_ram_size(main_ram_size: usize) -> Result<(), String> {
    let megabytes = main_ram_size / MEBIBYTE;
    let valid = main_ram_size.is_multiple_of(MEBIBYTE)
        && (megabytes == 1
            || (2..=X68K_MAX_MAIN_RAM_SIZE / MEBIBYTE).contains(&megabytes)
                && megabytes.is_multiple_of(2));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid X68000 main RAM size {main_ram_size}; expected 1, 2, 4, 6, 8, 10 or 12 MiB"
        ))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use common::{
        Bus, CpuMode, M68000AccessSize, M68000BusAccess, M68000CycleKind, M68000FunctionCode,
    };

    use super::{CGROM_SIZE, IPL_SIZE, X68kBus};
    use crate::{LoadedRoms, X68kModel};

    /// Builds a synthetic ROM set for bus tests.
    pub(crate) fn test_roms(model: X68kModel) -> LoadedRoms {
        let mut ipl = vec![0; IPL_SIZE];
        ipl[0x10000..0x10008].copy_from_slice(&[0, 0xC0, 0, 0, 0, 0xFE, 0, 8]);
        LoadedRoms {
            model,
            cgrom: vec![0xCC; CGROM_SIZE],
            ipl,
            internal_scsi: model.has_internal_scsi().then(|| vec![0x5A; 0x2000]),
            uses_compatibility_scsi: model == X68kModel::X68000Xvi,
        }
    }

    /// Audio sample rate used by bus tests.
    pub(crate) const TEST_SAMPLE_RATE: u32 = 48_000;

    /// Builds a bus from the synthetic ROM set.
    pub(crate) fn bus(model: X68kModel) -> X68kBus {
        X68kBus::new(model, CpuMode::High, test_roms(model), TEST_SAMPLE_RATE).unwrap()
    }

    /// Builds a machine from a synthetic or customized ROM set.
    pub(crate) fn machine(
        model: X68kModel,
        cpu_mode: CpuMode,
        roms: LoadedRoms,
    ) -> crate::X68kMachine {
        let bus = X68kBus::new(model, cpu_mode, roms, TEST_SAMPLE_RATE).unwrap();
        crate::X68kMachine::from_bus(model, cpu_mode, bus)
    }

    /// Builds a normal-cycle bus access.
    pub(crate) fn access(
        address: u32,
        size: M68000AccessSize,
        function_code: M68000FunctionCode,
    ) -> M68000BusAccess {
        M68000BusAccess {
            address,
            size,
            function_code,
            cycle_kind: M68000CycleKind::Normal,
        }
    }

    /// Programs a small display of the given character columns and rasters.
    pub(crate) fn tiny_display(bus: &mut X68kBus, width_characters: u16, height_rasters: u16) {
        let program = [
            width_characters + 6,
            0,
            0,
            width_characters,
            height_rasters + 2,
            0,
            0,
            height_rasters,
            0,
            height_rasters + 3,
        ];
        for (index, value) in program.into_iter().enumerate() {
            bus.crtc.write_register(index, value);
        }
        bus.contrast = 15;
    }

    /// Advances the bus in small cycle steps until the beam reaches a raster.
    pub(crate) fn advance_to_raster(bus: &mut X68kBus, raster: u16) {
        for _ in 0..1_000_000 {
            if bus.crtc.beam_position().raster == raster {
                return;
            }
            let next_cycle = bus.current_cycle + 20;
            bus.set_current_cycle(next_cycle);
            bus.synchronize_devices();
        }
        panic!("the beam never reached raster {raster}");
    }

    /// Advances the bus until the current frame completes and publishes.
    pub(crate) fn complete_frame(bus: &mut X68kBus) {
        let target = bus.crtc.frame_count() + 1;
        for _ in 0..1_000_000 {
            if bus.crtc.frame_count() >= target {
                return;
            }
            let next_cycle = bus.current_cycle + 20;
            bus.set_current_cycle(next_cycle);
            bus.synchronize_devices();
        }
        panic!("the frame never completed");
    }

    /// Writes one supervisor word through the validated bus path.
    pub(crate) fn write_word(bus: &mut X68kBus, address: u32, value: u16) {
        bus.m68000_write(
            access(
                address,
                M68000AccessSize::Word,
                M68000FunctionCode::SupervisorData,
            ),
            value,
        )
        .unwrap();
    }

    /// Reads one supervisor word through the validated bus path.
    pub(crate) fn read_word(bus: &mut X68kBus, address: u32) -> u16 {
        bus.m68000_read(access(
            address,
            M68000AccessSize::Word,
            M68000FunctionCode::SupervisorData,
        ))
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use common::{HostDateTime, Machine as _};

    use super::{
        test_support::{TEST_SAMPLE_RATE, access, bus, test_roms},
        *,
    };

    #[test]
    fn decoder_covers_every_boundary() {
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xBFFFFB),
            X68kRegion::MainRam
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xBFFFFF),
            X68kRegion::MainRam
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xC00000),
            X68kRegion::GraphicVram
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE80000),
            X68kRegion::Crtc
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE82000),
            X68kRegion::Palette
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE82400),
            X68kRegion::VideoController
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE88000),
            X68kRegion::Mfp
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE8A000),
            X68kRegion::Rtc
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE9A000),
            X68kRegion::Ppi
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xE9BFFF),
            X68kRegion::Ppi
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xEAFFFF),
            X68kRegion::BuiltinDevice
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xEB0000),
            X68kRegion::Sprite
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xEBFFFF),
            X68kRegion::Sprite
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xEC0000),
            X68kRegion::UserIo
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xED3FFF),
            X68kRegion::Sram
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xED4000),
            X68kRegion::Unmapped
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0xFFFFFF),
            X68kRegion::IplRom
        );
        assert_eq!(
            X68kBus::<NoTracing>::decode_region(0x1_000000),
            X68kRegion::MainRam
        );
    }

    #[test]
    fn explicit_main_ram_size_seeds_sram_and_bounds_ram_access() {
        let mut bus = X68kBus::<NoTracing>::with_main_ram_size(
            X68kModel::X68000,
            CpuMode::High,
            test_roms(X68kModel::X68000),
            TEST_SAMPLE_RATE,
            4 * MEBIBYTE,
        )
        .unwrap();
        let supervisor = M68000FunctionCode::SupervisorData;

        assert_eq!(bus.main_ram_size(), 4 * MEBIBYTE);
        assert_eq!(&bus.sram_data()[0x08..0x0C], &(4_u32 << 20).to_be_bytes());
        assert!(
            bus.m68000_write(access(0x3F_FFFF, M68000AccessSize::Byte, supervisor), 0x5A,)
                .is_ok()
        );
        assert!(
            bus.m68000_read(access(0x40_0000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
    }

    #[test]
    fn invalid_main_ram_sizes_are_rejected() {
        for size in [0, 3 * MEBIBYTE, 5 * MEBIBYTE, 13 * MEBIBYTE, MEBIBYTE + 512] {
            assert!(
                X68kBus::<NoTracing>::with_main_ram_size(
                    X68kModel::X68000,
                    CpuMode::High,
                    test_roms(X68kModel::X68000),
                    TEST_SAMPLE_RATE,
                    size,
                )
                .is_err(),
                "{size} should not be accepted"
            );
        }
    }

    #[test]
    fn ppi_accepts_ipl_initialization_and_reports_idle_joysticks() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_write(access(0xE9A007, M68000AccessSize::Byte, supervisor), 0x92)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE9A001, M68000AccessSize::Byte, supervisor)),
            Ok(0xFF)
        );
        assert_eq!(
            bus.m68000_read(access(0xE9A003, M68000AccessSize::Byte, supervisor)),
            Ok(0xFF)
        );
        bus.m68000_write(access(0xE9A005, M68000AccessSize::Byte, supervisor), 0x37)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE9A004, M68000AccessSize::Word, supervisor)),
            Ok(0xFF37)
        );
        assert_eq!(
            bus.m68000_read(access(0xE9A009, M68000AccessSize::Byte, supervisor)),
            Ok(0xFF)
        );
    }

    #[test]
    fn sprite_window_reaches_the_controller_and_masks_apply() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_write(access(0xEB0004, M68000AccessSize::Word, supervisor), 0xFFFF)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xEB0004, M68000AccessSize::Word, supervisor)),
            Ok(0xCFFF)
        );
        bus.m68000_write(access(0xEB8000, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        bus.m68000_write(access(0xEB8001, M68000AccessSize::Byte, supervisor), 0x009A)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xEB8000, M68000AccessSize::Word, supervisor)),
            Ok(0x9A9A)
        );
        assert_eq!(
            bus.m68000_read(access(0xEB0400, M68000AccessSize::Word, supervisor)),
            Ok(0xFFFF)
        );
        assert!(
            bus.m68000_read(access(
                0xEB0000,
                M68000AccessSize::Word,
                M68000FunctionCode::UserData
            ))
            .is_err()
        );
    }

    #[test]
    fn sprite_window_gates_on_the_crtc_memory_mode() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.crtc.write_register(20, 0x0016);
        assert!(
            bus.m68000_read(access(0xEB0000, M68000AccessSize::Word, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_read(access(0xEB8000, M68000AccessSize::Word, supervisor))
                .is_err()
        );
        assert!(
            bus.m68000_read(access(0xEB0840, M68000AccessSize::Word, supervisor))
                .is_err()
        );
        bus.m68000_write(access(0xEB0800, M68000AccessSize::Word, supervisor), 0x0123)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xEB0800, M68000AccessSize::Word, supervisor)),
            Ok(0x0123)
        );
        assert_eq!(
            bus.m68000_read(access(0xEB4000, M68000AccessSize::Word, supervisor)),
            Ok(0xFFFF)
        );
        bus.crtc.write_register(20, 0x0004);
        bus.m68000_write(access(0xEB0000, M68000AccessSize::Word, supervisor), 0x0055)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xEB0000, M68000AccessSize::Word, supervisor)),
            Ok(0x0055)
        );
    }

    #[test]
    fn reset_vectors_use_ipl_but_normal_zero_is_ram() {
        let mut bus = bus(X68kModel::X68000);
        for (address, expected) in [(0, 0x00C0), (2, 0), (4, 0x00FE), (6, 8)] {
            let mut vector_access = access(
                address,
                M68000AccessSize::Word,
                M68000FunctionCode::SupervisorProgram,
            );
            vector_access.cycle_kind = M68000CycleKind::ResetVector;
            assert_eq!(bus.m68000_read(vector_access), Ok(expected));
        }
        assert_eq!(
            bus.m68000_read(access(
                0,
                M68000AccessSize::Word,
                M68000FunctionCode::SupervisorData
            )),
            Ok(0)
        );
    }

    #[test]
    fn standard_and_enhanced_protection_use_function_codes() {
        let mut bus = bus(X68kModel::X68000);
        let user = M68000FunctionCode::UserData;
        let supervisor = M68000FunctionCode::SupervisorData;
        assert!(
            bus.m68000_read(access(0, M68000AccessSize::Byte, user))
                .is_err()
        );
        assert_eq!(
            bus.m68000_read(access(0, M68000AccessSize::Byte, supervisor)),
            Ok(0)
        );
        bus.m68000_write(access(0xEAFF81, M68000AccessSize::Byte, supervisor), 1)
            .unwrap();
        assert!(
            bus.m68000_read(access(0x200000, M68000AccessSize::Byte, user))
                .is_err()
        );
        assert_eq!(
            bus.m68000_read(access(0x240000, M68000AccessSize::Byte, user)),
            Ok(0)
        );
        assert!(
            bus.m68000_read(access(0xEAFF81, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
    }

    #[test]
    fn original_model_has_no_internal_scsi_window() {
        let mut original = bus(X68kModel::X68000);
        let mut super_model = bus(X68kModel::X68000Super);
        let read = access(
            0xFC0000,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        );
        assert!(original.m68000_read(read).is_err());
        assert_eq!(super_model.m68000_read(read), Ok(0x5A));
        let fill = access(
            0xFC2000,
            M68000AccessSize::Byte,
            M68000FunctionCode::SupervisorData,
        );
        assert_eq!(super_model.m68000_read(fill), Ok(0xFF));
    }

    #[test]
    fn every_enhanced_mask_controls_its_two_megabyte_block() {
        let mut bus = bus(X68kModel::X68000Super);
        for (index, register) in [0xEAFF81, 0xEAFF83, 0xEAFF85, 0xEAFF87, 0xEAFF89]
            .into_iter()
            .enumerate()
        {
            bus.m68000_write(
                access(
                    register,
                    M68000AccessSize::Byte,
                    M68000FunctionCode::SupervisorData,
                ),
                0x80,
            )
            .unwrap();
            let protected_address = 0x200000 + index as u32 * 0x200000 + 7 * 0x40000;
            assert!(
                bus.m68000_read(access(
                    protected_address,
                    M68000AccessSize::Byte,
                    M68000FunctionCode::UserData,
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn cpu_space_acknowledges_the_highest_routed_interrupt() {
        let mut bus = bus(X68kModel::X68000);
        bus.assert_interrupt(InterruptSource::Dmac, 0x44);
        bus.assert_interrupt(InterruptSource::Mfp, 0x46);
        assert_eq!(bus.m68000_interrupt_level(), 6);
        assert_eq!(bus.m68000_acknowledge_interrupt(6), 0x46);
        assert_eq!(bus.m68000_interrupt_level(), 3);
        assert_eq!(bus.m68000_acknowledge_interrupt(6), 0x18);
    }

    #[test]
    fn phase_two_register_lanes_and_mirrors_match_the_board() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        // R00 keeps the low byte with its hard-wired bit 0 set.
        bus.m68000_write(access(0xE80000, M68000AccessSize::Word, supervisor), 0x1234)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE80800, M68000AccessSize::Word, supervisor)),
            Ok(0x0035)
        );
        bus.m68000_write(access(0xE82202, M68000AccessSize::Word, supervisor), 0xABCD)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE82202, M68000AccessSize::Word, supervisor)),
            Ok(0xABCD)
        );
        assert!(
            bus.m68000_read(access(0xE88000, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
        bus.m68000_write(access(0xE88017, M68000AccessSize::Byte, supervisor), 0x40)
            .unwrap();
        assert_eq!(
            bus.m68000_read(access(0xE88057, M68000AccessSize::Byte, supervisor)),
            Ok(0x40)
        );
        assert!(
            bus.m68000_read(access(0xE88031, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
    }

    #[test]
    fn rtc_seeds_once_as_years_since_1980() {
        fn calendar() -> HostDateTime {
            HostDateTime {
                year: 2026,
                month: 7,
                day: 10,
                day_of_week: 5,
                hour: 12,
                minute: 34,
                second: 56,
            }
        }

        let bus = bus(X68kModel::X68000);
        let mut machine = crate::X68kMachine::from_bus(X68kModel::X68000, CpuMode::High, bus);
        machine.set_host_date_time_provider(calendar);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert_eq!(
            machine
                .bus
                .m68000_read(access(0xE8A017, M68000AccessSize::Byte, supervisor)),
            Ok(6)
        );
        assert_eq!(
            machine
                .bus
                .m68000_read(access(0xE8A057, M68000AccessSize::Byte, supervisor)),
            Ok(6)
        );
        assert!(
            machine
                .bus
                .m68000_read(access(0xE8A016, M68000AccessSize::Byte, supervisor))
                .is_err()
        );
    }

    #[test]
    fn mfp_interrupts_route_at_level_six() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (address, value) in [
            (0xE88007, 0x20),
            (0xE88013, 0x20),
            (0xE88017, 0x40),
            (0xE8801F, 1),
            (0xE88019, 1),
        ] {
            bus.m68000_write(access(address, M68000AccessSize::Byte, supervisor), value)
                .unwrap();
        }
        bus.set_current_cycle(20);
        bus.synchronize_devices();
        assert_eq!(bus.m68000_interrupt_level(), 6);
        assert_eq!(bus.m68000_acknowledge_interrupt(6), 0x4D);
        assert_eq!(bus.m68000_interrupt_level(), 0);
    }

    #[test]
    fn keyboard_bytes_cross_the_mfp_usart() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for (address, value) in [
            (0xE88021, 13),
            (0xE8802B, 1),
            (0xE8802D, 1),
            (0xE8E007, 8),
            (0xE8802F, 0x49),
        ] {
            bus.m68000_write(access(address, M68000AccessSize::Byte, supervisor), value)
                .unwrap();
        }
        bus.set_current_cycle(20_000);
        bus.synchronize_devices();
        bus.push_keyboard_scancode(0x1E);
        bus.set_current_cycle(40_000);
        bus.synchronize_devices();
        assert_ne!(
            bus.m68000_read(access(0xE8802B, M68000AccessSize::Byte, supervisor))
                .unwrap()
                & 0x80,
            0
        );
        assert_eq!(
            bus.m68000_read(access(0xE8802F, M68000AccessSize::Byte, supervisor)),
            Ok(0x1E)
        );
    }

    #[test]
    fn mfp_accesses_wait_four_cycles_each() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        assert_eq!(bus.drain_wait_cycles(), 0);
        for _ in 0..10 {
            bus.m68000_read(access(0xE88001, M68000AccessSize::Byte, supervisor))
                .unwrap();
        }
        assert_eq!(bus.drain_wait_cycles(), 40);
    }

    #[test]
    fn main_ram_word_accesses_wait_one_refresh_cycle_per_eight() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        for index in 0..8u32 {
            bus.m68000_read(access(index * 2, M68000AccessSize::Word, supervisor))
                .unwrap();
        }
        assert_eq!(bus.drain_wait_cycles(), 1);
        for index in 0..8u32 {
            bus.m68000_write(access(index * 2, M68000AccessSize::Word, supervisor), 0)
                .unwrap();
        }
        assert_eq!(bus.drain_wait_cycles(), 1);
    }

    #[test]
    fn region_waits_match_the_measured_penalty_table() {
        assert_eq!(cpu_access_wait_cycles(X68kRegion::MainRam), 0);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::VideoController), 1);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Crtc), 1);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Sram), 1);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::IplRom), 1);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::GraphicVram), 1);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Opm), 2);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Ioc), 2);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::TextVram), 2);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Palette), 3);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Mfp), 4);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Scc), 6);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Dmac), 15);
        assert_eq!(cpu_access_wait_cycles(X68kRegion::Midi), 0);
    }

    #[test]
    fn interrupt_acknowledge_and_legacy_bridge_stay_wait_free() {
        let mut bus = bus(X68kModel::X68000);
        bus.m68000_read(access(
            0x0E,
            M68000AccessSize::Word,
            M68000FunctionCode::CpuSpace,
        ))
        .unwrap();
        Bus::read_byte(&mut bus, 0xE88001);
        Bus::write_byte(&mut bus, 0x001000, 0x12);
        assert_eq!(bus.drain_wait_cycles(), 0);
    }

    #[test]
    fn failed_accesses_charge_no_wait() {
        let mut bus = bus(X68kModel::X68000);
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_read(access(0xED4000, M68000AccessSize::Byte, supervisor))
            .unwrap_err();
        bus.m68000_write(access(0xED4000, M68000AccessSize::Byte, supervisor), 0xFF)
            .unwrap_err();
        assert_eq!(bus.drain_wait_cycles(), 0);
    }

    /// The top word of the 12 MiB main-RAM area is ordinary RAM when installed,
    /// not an always-faulting aperture. Dai Makaimura's OPM driver writes there.
    #[test]
    fn top_of_main_ram_is_backed_when_installed() {
        let mut bus = bus(X68kModel::X68000);
        test_support::write_word(&mut bus, 0xBFFFFE, 0x1234);
        assert_eq!(test_support::read_word(&mut bus, 0xBFFFFE), 0x1234);
    }

    /// With less than 12 MiB installed the same address is unpopulated and still
    /// bus-errors, matching the expansion-ROM-pointer probe on real hardware.
    #[test]
    fn top_of_main_ram_faults_when_absent() {
        let mut bus: X68kBus = X68kBus::with_main_ram_size(
            X68kModel::X68000,
            CpuMode::High,
            test_roms(X68kModel::X68000),
            TEST_SAMPLE_RATE,
            2 * MEBIBYTE,
        )
        .unwrap();
        let supervisor = M68000FunctionCode::SupervisorData;
        bus.m68000_read(access(0xBFFFFE, M68000AccessSize::Word, supervisor))
            .unwrap_err();
        bus.m68000_write(access(0xBFFFFE, M68000AccessSize::Word, supervisor), 0)
            .unwrap_err();
    }
}
