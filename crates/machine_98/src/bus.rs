//! PC-9801 system bus implementing [`common::Bus`].
//!
//! Routes memory accesses to RAM/VRAM/ROM and I/O port accesses to
//! the appropriate peripheral (PIC, PIT, etc.).

mod bios;
mod dos_adapter;
mod fdc;
mod fdd_hle;
mod graphics;
mod hdd;
mod init;
mod io_read;
mod io_write;

use std::path::PathBuf;

use common::{
    CpuType, HostDateTimeProvider, MachineModel, StackVec, TraceAccessKind, TraceAccessWidth,
    TraceAddressSpace, TraceCall, TraceCallInterface, TraceCallPhase, TraceContext, TraceEvent,
    TraceEventKey, TraceField, TraceInterruptAction, TraceInterruptKind, TracePresentation,
    TraceValue, trace_id,
};
use device::{
    beeper::Beeper,
    cdrom::CdImage,
    cgrom::Cgrom,
    disk::HddImage,
    display_control::DisplayControl,
    egc::Egc,
    fdd320_ppi::Fdd320Ppi,
    fdd640k_hle::Fdd640kHle,
    floppy::FloppyImage,
    ga1280a::{Ga1280a, Ga1280aRenderSnapshot, Ga1280aState, is_ga1280a_port},
    grcg::Grcg,
    i8237_dma::I8237Dma,
    i8251_keyboard::I8251Keyboard,
    i8251_serial::I8251Serial,
    i8253_pit::I8253Pit,
    i8255_mouse_ppi::I8255MousePpi,
    i8255_system_ppi::I8255SystemPpi,
    i8259a_pic::I8259aPic,
    palette::Palette,
    pegc::Pegc,
    printer::Printer,
    sasi::SasiController,
    sdip::Sdip,
    sound_blaster_16::{SB16_PLATFORM_PC98, SoundBlaster16, SoundboardSb16Action},
    soundboard_14::{Soundboard14, Soundboard14Action},
    soundboard_26k::{Soundboard26k, Soundboard26kAction},
    soundboard_86::{Soundboard86, Soundboard86Action},
    upd765a_fdc::FloppyController,
    upd4990a_rtc::Upd4990aRtc,
    upd7220_gdc::{DISPLAY_MODE_GRAPHICS, Gdc},
    upd52611_crtc::Upd52611Crtc,
};
use software_renderer::{
    Ga1280aCursorRenderInputs, Ga1280aRenderInputs, Ga1280aRenderMode, GdcGraphicsInput,
    GraphicsInput, PegcRenderInputs, RenderInputs, SoftwareRenderer, compose_ga1280a,
};

use crate::{
    NoTrace, TraceSink,
    config::ClockConfig,
    memory::Pc9801Memory,
    scheduler::{Event98, Pc98Scheduler},
};

/// Traces a handled main-CPU bus access when tracing is enabled.
macro_rules! trace_access {
    (
        $sink:ty,
        $bus:expr,
        $space:ident,
        ($kind:expr),
        $address:expr,
        $width:ident,
        $value:expr $(,)?
    ) => {
        trace_access!(@emit, $sink, $bus, $space, $kind, $address, $width, $value);
    };
    (
        $sink:ty,
        $bus:expr,
        $space:ident,
        $kind:ident,
        $address:expr,
        $width:ident,
        $value:expr $(,)?
    ) => {
        trace_access!(
            @emit,
            $sink,
            $bus,
            $space,
            TraceAccessKind::$kind,
            $address,
            $width,
            $value
        );
    };
    (
        @emit,
        $sink:ty,
        $bus:expr,
        $space:ident,
        $kind:expr,
        $address:expr,
        $width:ident,
        $value:expr
    ) => {
        if <$sink as TraceSink>::ENABLED {
            ($bus).tracer.trace(
                TraceContext::main_cpu(
                    ($bus).current_cycle,
                    Some(u64::from(($bus).clocks.cpu_clock_hz)),
                ),
                TraceEvent::access(
                    TraceAddressSpace::$space,
                    $kind,
                    u64::from($address),
                    TraceAccessWidth::$width,
                    Some(u64::from($value)),
                    true,
                ),
            );
        }
    };
}

/// Text RAM (0xA0000-0xA3FFF) access wait penalty in CPU cycles.
const TRAM_WAIT_CYCLES: i64 = 1;

/// Graphics VRAM (0xA8000-0xBFFFF) access wait penalty in CPU cycles (display period).
/// During VSYNC blanking, this drops to 1 cycle.
const VRAM_WAIT_CYCLES: i64 = 6;

/// GRCG VRAM access wait penalty in CPU cycles (display period).
/// During VSYNC blanking, this drops to 1 cycle.
/// Used for TCR reads, TDW writes, and RMW writes. RMW reads use VRAM_WAIT_CYCLES instead.
const GRCG_WAIT_CYCLES: i64 = 8;

/// I/O bus access wait penalty in CPU cycles.
/// Each byte-sized I/O read or write incurs this penalty.
const IO_WAIT_CYCLES: i64 = 1;

const DIGITAL_GRAPHICS_PALETTE_BASE: usize = 8;

/// DMA access control register (port 0x0439) default: 20-bit DMA mask.
///
/// Used by 8/10 MHz machines (VM, VX). On 386+ machines (RA, PC-9821),
/// the register starts at 0x00 (full 24/32-bit DMA addressing).
/// Ref: undoc98 `io_dma.txt` (port 0x0439).
const DMA_ACCESS_CTRL_20BIT: u8 = 0x04;

/// System status register (port 0xF0 read) default for a minimal VM config.
/// All bits clear = no sound board, no IDE interface installed.
/// Ref: undoc98 `io_cpu.txt` (port 0xF0)
const SYSTEM_STATUS_DEFAULT: u8 = 0x00;

/// Normal/hi-res mode detection register (port 0x0431 read).
/// Bit 2 = 1 means normal mode (640x400/640x200).
/// Hi-res mode (1120x750) is only on PC-H98, PC-98XA/XL/RL, and some PC-9821 models.
/// Ref: undoc98 `io_hires.txt` (port 0x0431)
const MODE_DETECT_NORMAL: u8 = 0x04;

/// Sets a PIC IRQ line and traces state transitions.
fn update_pic_irq<T: TraceSink>(
    pic: &mut I8259aPic,
    tracer: &mut T,
    current_cycle: u64,
    cpu_clock_hz: u32,
    irq: u8,
    asserted: bool,
) {
    let changed = if asserted {
        pic.set_irq(irq)
    } else {
        pic.clear_irq(irq)
    };
    if T::ENABLED && changed {
        let action = if asserted {
            TraceInterruptAction::Assert
        } else {
            TraceInterruptAction::Clear
        };
        tracer.trace(
            TraceContext::main_cpu(current_cycle, Some(u64::from(cpu_clock_hz))),
            TraceEvent::maskable_interrupt(
                trace_id::controller::PC98_PIC,
                u16::from(irq),
                action,
                None,
            ),
        );
    }
}

fn pack_rgba(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16) | 0xFF00_0000
}

fn pack_fixed_color(index: u8) -> u32 {
    let blue = if index & 0x01 != 0 { 0xFF } else { 0 };
    let red = if index & 0x02 != 0 { 0xFF } else { 0 };
    let green = if index & 0x04 != 0 { 0xFF } else { 0 };

    pack_rgba(red, green, blue)
}

fn digital_palette_register_index(color_index: usize) -> usize {
    match color_index & 0x03 {
        0 => 3,
        1 => 1,
        2 => 2,
        3 => 0,
        _ => unreachable!(),
    }
}

fn pack_digital_graphics_color(digital_palette: &[u8; 4], color_index: usize) -> u32 {
    let register_index = digital_palette_register_index(color_index);
    let packed_pair = digital_palette[register_index];
    let packed_color = if color_index < 4 {
        packed_pair >> 4
    } else {
        packed_pair & 0x0F
    };

    pack_fixed_color(packed_color)
}

fn digital_monochrome_mask(digital_palette: &[u8; 4]) -> u32 {
    let mut mask = 0;
    for color_index in 0..4 {
        let packed_pair = digital_palette[digital_palette_register_index(color_index)];
        if packed_pair & 0x40 != 0 {
            mask |= (1 << color_index) | (1 << (color_index + 8));
        }
        if packed_pair & 0x04 != 0 {
            mask |= (1 << (color_index + 4)) | (1 << (color_index + 12));
        }
    }
    mask
}

/// 1MB FDC external circuit input register value (port 0x0094 read).
/// Bit 6: FINT0 = 1 (fixed for dual-mode FD I/F).
/// Bit 2: TYP0 = 1 (internal drives are #1, #2, DIP SW 1-4 OFF).
/// Ref: undoc98 `io_fdd.txt`
const FDC_1MB_INPUT_REGISTER: u8 = 0x44;

/// 640KB FDC external circuit input register value (port 0x00CC read).
/// Bit 6: FINT0 = 1 (fixed for dual-mode FD I/F).
/// Bit 5: DMACH = 1 (fixed for 640KB I/F mode).
/// Bit 4: RDY = 1 (drive ready).
/// Bit 2: TYP0 = 1 (internal drives are #1, #2, DIP SW 1-4 OFF).
/// Ref: undoc98 `io_fdd.txt`
const FDC_640K_INPUT_REGISTER: u8 = 0x74;

/// FDC media read mask: bits 0-1 from stored value, upper bits fixed at 1.
const FDC_MEDIA_READ_FIXED_BITS: u8 = 0xF8;

/// Interrupt delay after data transfer completes.
const INTERRUPT_DELAY_CYCLES: u64 = 512;

/// Mouse interrupt timer register default (port 0xBFDB).
///
/// Lower 2 bits select periodic interrupt rate: 0x00 = 120 Hz (default).
const MOUSE_TIMER_DEFAULT_SETTING: u8 = 0x00;

/// Mouse timer IRQ line on PC-98 (slave IR5 -> INT 15h).
const MOUSE_TIMER_IRQ_LINE: u8 = 13;

/// MPU timer IRQ line.
const MPU_IRQ_LINE: u8 = 3;

/// Keyboard shift/code table offset within the BIOS code segment (FD80h).
///
/// PC-9801F uses an earlier table layout than the VM and later models.
const KEYBOARD_ROM_OFFSET_F: usize = 0x0A58;
const KEYBOARD_ROM_OFFSET_VM: usize = 0x0B28;

/// Graphics VRAM bytes per page for the B/R/G planes.
const GRAPHICS_PAGE_SIZE_BYTES: usize = 0x18000;

/// E-plane VRAM bytes per page.
const E_PLANE_PAGE_SIZE_BYTES: usize = 0x8000;

/// Boot device selection for the HLE bootstrap.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum BootDevice {
    /// Try all devices in standard order: FDD 0-1, CD-ROM, SASI HDD, IDE HDD, HLE DOS.
    #[default]
    Auto,
    /// Boot from FDD drive 0 only.
    Fdd1,
    /// Boot from FDD drive 1 only.
    Fdd2,
    /// Boot from HDD drive 0 (SASI or IDE depending on machine model).
    Hdd1,
    /// Boot from HDD drive 1 (SASI or IDE depending on machine model).
    Hdd2,
    /// Skip all disk boot attempts, go straight to HLE Neetan DOS.
    Dos,
}

impl std::fmt::Display for BootDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Fdd1 => f.write_str("fdd1"),
            Self::Fdd2 => f.write_str("fdd2"),
            Self::Hdd1 => f.write_str("hdd1"),
            Self::Hdd2 => f.write_str("hdd2"),
            Self::Dos => f.write_str("dos"),
        }
    }
}

impl std::str::FromStr for BootDevice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "fdd1" => Ok(Self::Fdd1),
            "fdd2" => Ok(Self::Fdd2),
            "hdd1" => Ok(Self::Hdd1),
            "hdd2" => Ok(Self::Hdd2),
            "dos" => Ok(Self::Dos),
            _ => Err(format!(
                "unknown boot device '{s}', expected auto, fdd1, fdd2, hdd1, hdd2 or dos"
            )),
        }
    }
}

/// PC-9801 system bus.
pub struct Pc9801Bus<T: TraceSink = NoTrace> {
    pub(crate) current_cycle: u64,
    pub(crate) next_event_cycle: u64,
    pub(crate) nmi_enabled: bool,
    memory: Pc9801Memory,
    pub(crate) pic: I8259aPic,
    pub(crate) scheduler: Pc98Scheduler,
    clocks: ClockConfig,
    pit: I8253Pit,
    dma: I8237Dma,
    keyboard: I8251Keyboard,
    /// Scan code already consumed from port 41h by a guest INT 09h pre-handler.
    keyboard_chained_raw_code: Option<u8>,
    serial: I8251Serial,
    gdc_master: Gdc,
    gdc_slave: Gdc,
    /// PC-98 floppy controller (both FDC interfaces + drive storage).
    floppy: FloppyController,
    /// PC-9801-09-style 640KB FDD BIOS HLE extension.
    fdd640k_hle: Fdd640kHle,
    fdd320_ppi: Fdd320Ppi,
    system_ppi: I8255SystemPpi,
    printer: Printer,
    display_control: DisplayControl,
    cgrom: Cgrom,
    crtc: Upd52611Crtc,
    grcg: Grcg,
    egc: Egc,
    pegc: Pegc,
    palette: Palette,
    soundboard_14: Option<Soundboard14>,
    soundboard_26k: Option<Soundboard26k>,
    soundboard_86: Option<Soundboard86>,
    sound_blaster_16: Option<SoundBlaster16<SB16_PLATFORM_PC98>>,
    ga1280a: Option<Ga1280a>,
    beeper: Beeper,
    rtc: Upd4990aRtc,
    /// Returns the current host local time as 6-byte BCD:
    /// `[year, month<<4|day_of_week, day, hour, minute, second]`.
    host_date_time_provider: HostDateTimeProvider,
    /// MPU-PC98II MIDI interface (C-Bus, default base 0xE0D0).
    mpu401: device::mpu401::Mpu401,
    /// MT-32 sound module (optional, requires munt).
    #[cfg(feature = "mt32")]
    mt32: Option<device::mt32::Mt32>,
    /// SC-55 sound module (optional, requires Nuked-SC55).
    #[cfg(feature = "sc55")]
    sc55: Option<device::sc55::Sc55>,
    mouse_ppi: I8255MousePpi,
    /// Mouse interrupt timer register (port 0xBFDB).
    mouse_timer_setting: u8,
    /// PC-9801-27 SASI hard disk controller.
    sasi: SasiController,
    /// PC-98 IDE (ATA) hard disk controller.
    ide: device::ide::IdeController,
    /// Software DIP Switch (SDIP) - NVRAM configuration on PC-9821.
    sdip: Sdip,
    /// BIOS HLE trap controller.
    bios: device::bios::BiosController,
    /// Whether the BIOS interval timer single-shot service is currently armed.
    bios_interval_timer_active: bool,
    /// Cached CPU mode for deciding whether PIT IRQ0 still flows through BIOS INT 08h HLE.
    current_cpu_protected_mode: bool,
    a20_enabled: bool,
    machine_model: MachineModel,
    reset_pending: bool,
    /// Set when the guest triggers a SYSTEM SHUTDOWN (SHUT0=1, SHUT1=0 when
    /// port 0xF0 is written). The host application should exit cleanly.
    shutdown_requested: bool,
    /// Set when a cold reset (port 0xF0 write) has occurred. The HLE
    /// VEC_ITF_ENTRY handler checks this to decide whether to reinitialize
    /// all devices to post-BIOS state. Cleared after the HLE handler processes it.
    needs_full_reinit: bool,
    /// Warm-reset context captured at the moment of the port 0xF0 write.
    /// On real hardware the CPU stops immediately; in our emulator the CPU
    /// continues until the machine loop checks, so we snapshot the state.
    warm_reset_context: Option<(u16, u16, u16, u16)>,
    /// CPU-side software renderer that produces the composed framebuffer
    /// uploaded by the graphics engine..
    software_renderer: Box<SoftwareRenderer>,
    /// Active output width in pixels from the most recent composed frame.
    display_width: u32,
    /// Active output height in pixels from the most recent composed frame.
    display_height: u32,
    /// Number assigned to the next published frame.
    presented_frames: u64,
    /// DMA access control register (port 0x0439). Bit 2: mask DMA above 1MB.
    dma_access_ctrl: u8,
    /// VRAM/EMS bank register (write-only via port 0x043F).
    vram_ems_bank: u8,
    /// RAM window register (write-only via port 0x0461).
    ram_window: u8,
    /// 15M hole control register (port 0x043B). Controls F00000-FFFFFF accessibility.
    hole_15m_control: u8,
    /// Protected memory registration register (port 0x0567).
    protected_memory_max: u8,
    /// Whether NEC B-bank EMS (port 0x043F bit 1) is supported.
    /// Switches B0000-BFFFF between graphics VRAM and extended RAM at 0x100000.
    /// Present on RA and later 386+ models.
    /// Ref: undoc98 memsys.txt line 1767, io_mem.txt port 0x043F.
    b_bank_ems: bool,
    /// Whether the 16-color graphics extension (E-plane VRAM) is installed.
    graphics_extension_enabled: bool,
    pending_wait_cycles: i64,
    /// Undocumented RTC control/mode latch (port 0x0022).
    rtc_control_22: u8,
    /// Key-down sense latch (port 0x00EC).
    key_sense_0ec: u8,
    /// Expansion-slot socket processing latch (port 0x043A).
    external_interrupt_43a: u8,
    /// Extended video flip-flop index register (port 0x09A0).
    video_ff2_index: u8,
    /// Window Accelerator Board index register (port 0x0FAA).
    /// Used on PC-9821 for built-in graphics accelerator control.
    wab_index: u8,
    /// Window Accelerator Board data registers (indexed by `wab_index`).
    wab_data: [u8; 8],
    /// Display output relay control (port 0x0FAC).
    wab_relay: u8,
    /// CPU mode / wait control register (port 0x0534).
    cpu_mode_534: u8,
    /// SIMM memory controller address register (port 0x0530).
    /// Bit 7: 1=limit address, 0=base address. Bits 3-0: socket number.
    /// Ref: undoc98 `io_mem.txt` (port 0x0530)
    simm_address_register: u8,
    /// SIMM memory controller data (indexed by simm_address_register).
    /// 16 sockets × 2 (base + limit) = 32 entries.
    simm_data: [u8; 32],
    /// Memory bank switching register (port 0x063C).
    /// Ref: undoc98 `io_mem.txt` (port 0x063C)
    memory_bank_063c: u8,
    /// CPU/cache control register (port 0x063F).
    /// Ref: undoc98 `io_mem.txt` (port 0x063F)
    cache_control_063f: u8,
    /// Current text RAM access wait penalty in CPU cycles.
    /// Switched between display-period and VSYNC-blanking values by the
    /// GdcVsync / GdcDisplayStart event handlers.
    tram_wait: i64,
    /// Current graphics VRAM access wait penalty in CPU cycles.
    vram_wait: i64,
    /// Current GRCG VRAM access wait penalty in CPU cycles.
    grcg_wait: i64,
    /// Whether the one-shot post-BIOS timer fixup has been applied.
    ///
    /// On a real PC-98, the BIOS (or the application via INT 1Ch AH=02/03)
    /// unmasks IRQ 0 in the PIC to start the system timer.
    tracer: T,
    /// HLE BIOS: per-drive seek cylinder position.
    fdd_seek_cylinder: [u8; 4],
    /// HLE BIOS: per-drive rotating sector ID cursor for READ ID.
    fdd_read_id_index: [usize; 4],
    /// Cached CR0 from the CPU, set before HLE dispatch.
    hle_cr0: u32,
    /// Cached CR3 from the CPU, set before HLE dispatch.
    hle_cr3: u32,
    /// NEETAN DOS HLE DOS instance. `None` when running with real DOS media.
    boot_device: BootDevice,
    dos: Option<dos::NeetanDos>,
    /// Whether EMS expanded memory is enabled for the HLE DOS.
    ems_enabled: bool,
    /// Whether XMS extended memory is enabled for the HLE DOS.
    xms_enabled: bool,
    /// Whether 32-bit XMS super functions (0x88-0x8F) are enabled.
    xms_32_enabled: bool,
    /// /HMAMIN= threshold in KB for XMS Request HMA (XMS.txt priority).
    xms_hmamin_kb: u16,
    /// Optional sink that receives JIS codes observed at the CGROM glyph
    /// fetch port (0xA9). Host-side runtime configuration; intentionally
    /// excluded from save/load.
    text_extractor: Option<Box<dyn common::TextExtractor>>,
}

impl<T: TraceSink> Pc9801Bus<T> {
    /// Updates the cached CPU mode used for IRQ0 / BIOS interval-timer routing.
    pub(crate) fn set_cpu_protected_mode_enabled(&mut self, enabled: bool) {
        self.current_cpu_protected_mode = enabled;
    }

    /// Sets the boot device for the HLE bootstrap.
    pub fn set_boot_device(&mut self, device: BootDevice) {
        self.boot_device = device;
    }

    /// Enables or disables EMS expanded memory for the HLE DOS.
    pub fn set_ems_enabled(&mut self, enabled: bool) {
        self.ems_enabled = enabled;
    }

    /// Enables or disables XMS extended memory for the HLE DOS.
    pub fn set_xms_enabled(&mut self, enabled: bool) {
        self.xms_enabled = enabled;
    }

    /// Sets the XMS /HMAMIN= threshold in KB for the HLE DOS.
    pub fn set_xms_hmamin_kb(&mut self, hmamin_kb: u16) {
        self.xms_hmamin_kb = hmamin_kb;
    }

    /// Enables or disables 32-bit XMS super functions (0x88-0x8F).
    pub fn set_xms_32_enabled(&mut self, enabled: bool) {
        self.xms_32_enabled = enabled;
    }

    /// Enables the NEETAN DOS HLE DOS subsystem.
    ///
    /// When enabled, DOS interrupt vectors (INT 20h-2Ah, 2Fh, 33h, DCh) are
    /// routed to the built-in Rust implementation instead of being passed
    /// through to a real DOS loaded from disk.
    pub fn enable_neetan_dos(&mut self) {
        let mut dos = dos::NeetanDos::new();
        dos.set_host_date_time_provider(self.host_date_time_provider);
        self.dos = Some(dos);
    }

    /// Loads BIOS ROM data (mapped at E8000-FFFFF, up to 96 KB).
    ///
    /// Clears the IVT and BDA entries that were populated for the embedded
    /// stub BIOS during construction. A real BIOS ROM sets these up during
    /// its own boot sequence; stale stub entries would point to wrong handler
    /// offsets and cause crashes on interrupt.
    pub fn load_bios_rom(&mut self, data: &[u8]) {
        self.memory.load_rom(data);
        self.memory.state.ram[0x0000..0x0400].fill(0);
        self.memory.state.ram[0x0400] = 0;
        self.memory.state.ram[0x0480] = 0;
        self.memory.state.ram[0x0500] = 0;
        self.memory.state.ram[0x0501] = 0;
        // Reset HLE-specific state that would interfere with real BIOS execution.
        // Shadow RAM redirect must be off so the CPU reads code from ROM.
        self.memory.set_shadow_control(0);
        self.vram_ems_bank = 0;
        self.protected_memory_max = 0;
    }

    /// Loads a V98-format font ROM into the CGROM buffer and propagates the
    /// new bytes to the software renderer.
    pub fn load_font_rom(&mut self, data: &[u8]) {
        self.memory.load_font_rom(data);
        self.software_renderer
            .update_font_rom(self.memory.font_rom_data());
    }

    /// Loads the PC-9801-26K sound ROM (16 KB at CC000-CFFFF).
    ///
    /// Pass `Some(data)` for a full ROM dump, or `None` to install a
    /// minimal stub that provides a no-op INT D2h handler.
    pub fn load_sound_rom(&mut self, data: Option<&[u8]>) {
        self.memory.load_sound_rom(data);
    }

    /// Inserts a floppy disk image into the specified drive (0-3).
    pub fn insert_floppy(&mut self, drive: usize, image: FloppyImage, path: Option<PathBuf>) {
        let install_fdd640k_hle = self.machine_model == MachineModel::PC9801F;
        self.floppy.insert_drive(drive, image, path);
        if install_fdd640k_hle {
            self.fdd640k_hle.install_rom();
        }
        let access_page = self.access_page_index();
        let b_bank_ems = self.b_bank_ems;
        let vram_ems_bank = self.vram_ems_bank;
        if let Some(dos) = self.dos.as_mut() {
            let memory = dos_adapter::DosMemoryAccess::new(
                &mut self.memory,
                access_page,
                b_bank_ems,
                vram_ems_bank,
            );
            dos.invalidate_drive_caches(&memory, 0x90 | drive as u8);
        }
    }

    /// Returns a reference to the disk image in the given drive, if present.
    pub fn floppy_disk(&self, drive: usize) -> Option<&FloppyImage> {
        self.floppy.drive(drive)
    }

    /// Returns whether the disk in the given drive has been modified.
    pub fn is_floppy_dirty(&self, drive: usize) -> bool {
        self.floppy.is_drive_dirty(drive)
    }

    /// Ejects the floppy disk from the specified drive, flushing if dirty.
    pub fn eject_floppy(&mut self, drive: usize) {
        self.floppy.eject_drive(drive);
        let access_page = self.access_page_index();
        let b_bank_ems = self.b_bank_ems;
        let vram_ems_bank = self.vram_ems_bank;
        if let Some(dos) = self.dos.as_mut() {
            let memory = dos_adapter::DosMemoryAccess::new(
                &mut self.memory,
                access_page,
                b_bank_ems,
                vram_ems_bank,
            );
            dos.invalidate_drive_caches(&memory, 0x90 | drive as u8);
        }
    }

    /// Writes the floppy image back to its file if it has been modified.
    pub fn flush_floppy(&mut self, drive: usize) {
        self.floppy.flush_drive(drive);
    }

    /// Flushes all dirty floppy images to disk.
    pub fn flush_all_floppies(&mut self) {
        self.floppy.flush_all_drives();
    }

    /// Inserts a hard disk image into the specified drive (0-1).
    pub fn insert_hdd(&mut self, drive: usize, image: HddImage, path: Option<PathBuf>) {
        match self.machine_model {
            MachineModel::PC9801F
            | MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA => {
                self.sasi.insert_drive(drive, image, path);
            }
            MachineModel::PC9821AS | MachineModel::PC9821AP => {
                self.ide.insert_drive(drive, image, path);
                if self
                    .ide
                    .drive_geometry(drive)
                    .is_some_and(|g| g.sector_size == 256)
                {
                    self.sdip.set_front_bank_bit(0, 6, false);
                }
            }
        }
        let access_page = self.access_page_index();
        let b_bank_ems = self.b_bank_ems;
        let vram_ems_bank = self.vram_ems_bank;
        if let Some(dos) = self.dos.as_mut() {
            let memory = dos_adapter::DosMemoryAccess::new(
                &mut self.memory,
                access_page,
                b_bank_ems,
                vram_ems_bank,
            );
            dos.invalidate_drive_caches(&memory, 0x80 | drive as u8);
        }
    }

    /// Writes the HDD image back to its file if it has been modified.
    pub fn flush_hdd(&mut self, drive: usize) {
        match self.machine_model {
            MachineModel::PC9801F
            | MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA => {
                self.sasi.flush_drive(drive);
            }
            MachineModel::PC9821AS | MachineModel::PC9821AP => {
                self.ide.flush_drive(drive);
            }
        }
    }

    /// Flushes all dirty HDD images to disk.
    pub fn flush_all_hdds(&mut self) {
        match self.machine_model {
            MachineModel::PC9801F
            | MachineModel::PC9801VM
            | MachineModel::PC9801VX
            | MachineModel::PC9801RS
            | MachineModel::PC9801RA => {
                self.sasi.flush_all_drives();
            }
            MachineModel::PC9821AS | MachineModel::PC9821AP => {
                self.ide.flush_all_drives();
            }
        }
    }

    /// Inserts a CD-ROM image into the IDE controller (channel 1).
    /// Only available on PC-9821 models with IDE.
    pub fn insert_cdrom(&mut self, image: CdImage) {
        if self.machine_model.has_ide() {
            self.ide.insert_cdrom(image);
        }
    }

    /// Ejects the CD-ROM image from the IDE controller.
    pub fn eject_cdrom(&mut self) {
        if self.machine_model.has_ide() {
            self.ide.eject_cdrom();
        }
    }

    /// Returns true if a CD-ROM image is loaded.
    pub fn has_cdrom(&self) -> bool {
        self.ide.has_cdrom()
    }

    /// Returns the current CD audio playback state and positions.
    pub fn cd_audio_status(&self) -> Option<common::CdAudioStatus> {
        if !self.ide.has_cdrom() {
            return None;
        }

        let player = self.ide.cd_audio_player();
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

    /// Attaches a file handle for printer output.
    pub fn attach_printer(&mut self, file: std::fs::File) {
        self.printer.attach(file);
    }

    /// Flushes the printer output file.
    pub fn flush_printer(&mut self) {
        self.printer.flush();
    }

    /// Installs a text extractor sink that receives JIS codes observed at
    /// the CGROM glyph fetch port.
    pub fn install_text_extractor(&mut self, extractor: Box<dyn common::TextExtractor>) {
        self.text_extractor = Some(extractor);
    }

    /// Drives the installed text extractor's heartbeat. No-op when no
    /// extractor is installed.
    pub fn tick_text_extractor(&mut self) {
        if let Some(extractor) = self.text_extractor.as_deref_mut() {
            extractor.tick();
        }
    }

    /// Injects one keyboard scan code and raises IRQ1.
    pub fn push_keyboard_scancode(&mut self, code: u8) {
        self.keyboard.push_scancode(code);
        self.keyboard_chained_raw_code = None;
        self.raise_pic_irq(1);
    }

    /// Injects one serial byte and raises IRQ4.
    pub fn push_serial_byte(&mut self, data: u8) {
        self.serial.push_received_byte(data);
        self.raise_pic_irq(4);
    }

    /// Injects mouse movement deltas for the current frame.
    pub fn push_mouse_delta(&mut self, dx: i16, dy: i16) {
        self.mouse_ppi.sync_frame(dx, dy, self.current_cycle);
    }

    /// Updates mouse button state.
    pub fn set_mouse_buttons(&mut self, left: bool, right: bool, middle: bool) {
        self.mouse_ppi.set_buttons(left, right, middle);
    }

    /// Sets or clears a single bit in DIP switch 2.
    pub fn set_dip_switch_2_bit(&mut self, bit: u8, value: bool) {
        if value {
            self.system_ppi.state.dip_switch_2 |= 1 << bit;
        } else {
            self.system_ppi.state.dip_switch_2 &= !(1 << bit);
        }
    }

    /// Configures the GDC clock to 5 MHz (400-line graphics mode).
    ///
    /// Equivalent to setting DIP switch 2-8 to ON on real hardware.
    pub fn set_gdc_clock_5mhz(&mut self) {
        self.system_ppi.state.dip_switch_2 &= !0x80;
        if self.machine_model.has_sdip() {
            // SDIP register 1 (0x851E) bit 7 = GDC clock.
            // Clearing it selects 5 MHz; parity at bit 4 is recomputed.
            self.sdip.set_front_bank_bit(1, 7, false);
        }
        self.memory.state.ram[0x054C] &= !0x40;
        self.memory.state.ram[0x054D] |= 0x20;
        self.gdc_slave.state.lines_per_row = 1;
        self.display_control.state.mode2 |= 0x0600;
    }

    /// Returns the CPU clock frequency in Hz.
    pub fn cpu_clock_hz(&self) -> u32 {
        self.clocks.cpu_clock_hz
    }

    /// Returns the PIT clock frequency in Hz.
    pub fn pit_clock_hz(&self) -> u32 {
        self.clocks.pit_clock_hz
    }

    /// Returns a reference to the tracer.
    pub fn tracer(&self) -> &T {
        &self.tracer
    }

    /// Returns a mutable reference to the tracer.
    pub fn tracer_mut(&mut self) -> &mut T {
        &mut self.tracer
    }

    /// Sets a PIC IRQ line and traces state transitions.
    pub(crate) fn set_pic_irq(&mut self, irq: u8, asserted: bool) {
        update_pic_irq(
            &mut self.pic,
            &mut self.tracer,
            self.current_cycle,
            self.clocks.cpu_clock_hz,
            irq,
            asserted,
        );
    }

    /// Raises a PIC IRQ line and traces the transition.
    pub(crate) fn raise_pic_irq(&mut self, irq: u8) {
        self.set_pic_irq(irq, true);
    }

    /// Clears a PIC IRQ line and traces the transition.
    pub(crate) fn clear_pic_irq(&mut self, irq: u8) {
        self.set_pic_irq(irq, false);
    }

    pub(crate) fn trace_call(
        &mut self,
        provider: &'static str,
        interface: TraceCallInterface,
        function: Option<u64>,
        subfunction: Option<u64>,
        phase: TraceCallPhase,
        result: Option<u64>,
    ) {
        if !T::ENABLED
            || !self
                .tracer
                .interested(TraceEventKey::Call { provider, phase })
        {
            return;
        }
        let mut fields = StackVec::<TraceField<'_>, 3>::new();
        if let Some(function) = function {
            fields.push(TraceField {
                name: trace_id::field::FUNCTION,
                value: TraceValue::Unsigned(function),
            });
        }
        if let Some(subfunction) = subfunction {
            fields.push(TraceField {
                name: trace_id::field::SUBFUNCTION,
                value: TraceValue::Unsigned(subfunction),
            });
        }
        if let Some(result) = result {
            fields.push(TraceField {
                name: trace_id::field::RESULT,
                value: TraceValue::Unsigned(result),
            });
        }
        self.tracer.trace(
            TraceContext::main_cpu(
                self.current_cycle,
                Some(u64::from(self.clocks.cpu_clock_hz)),
            ),
            TraceEvent::Call(TraceCall {
                provider,
                interface,
                phase,
                fields: &fields,
            }),
        );
    }

    /// Returns and clears the CPU reset pending flag. If a warm-reset
    /// context was captured at the time of the port 0xF0 write, it is
    /// returned as `Some((ss, sp, cs, ip))`.
    pub fn take_reset_pending(&mut self) -> Option<Option<(u16, u16, u16, u16)>> {
        if std::mem::replace(&mut self.reset_pending, false) {
            Some(self.warm_reset_context.take())
        } else {
            None
        }
    }

    /// Returns `true` if the guest triggered a SYSTEM SHUTDOWN
    /// (SHUT0=1, SHUT1=0 when port 0xF0 was written).
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Reads a 16-bit little-endian word directly from physical memory
    /// without side effects.
    fn read_word_direct(&self, physical_address: u32) -> u16 {
        let lo = self.read_byte_direct(physical_address) as u16;
        let hi = self.read_byte_direct(physical_address + 1) as u16;
        lo | (hi << 8)
    }

    /// Selects the ITF ROM bank for the F8000-FFFFF window.
    pub fn select_rom_bank_itf(&mut self) {
        self.memory.select_banked_rom_window(false);
    }

    /// Returns the CPU type configured for this bus.
    pub fn cpu_type(&self) -> CpuType {
        self.machine_model.cpu_type()
    }

    /// Returns the machine model configured for this bus.
    pub fn machine_model(&self) -> MachineModel {
        self.machine_model
    }

    /// Enables CG RAM mode (VX+). All character codes become writable.
    fn set_cg_ram(&mut self, enabled: bool) {
        self.cgrom.state.cg_ram = enabled;
    }

    /// Sets the host local time provider for the µPD4990A RTC.
    ///
    /// Also updates Memory Switch 8 (`A000:3FFEh`) with the BCD year byte,
    /// since the µPD1990A (used by VM-class machines) has no year register
    /// and the BIOS reads the year from the memory switch instead.
    pub(crate) fn set_host_date_time_provider(&mut self, provider: HostDateTimeProvider) {
        self.host_date_time_provider = provider;
        self.memory.state.text_vram[0x3FFE] = provider().to_bcd_bytes()[0];
    }

    /// Enables/disables the 16-color graphics extension board.
    pub fn set_graphics_extension_enabled(&mut self, enabled: bool) {
        self.graphics_extension_enabled = enabled;
        self.system_ppi.set_graphics_extension_bit(enabled);
        self.update_plane_e_mapping();
    }

    /// Installs an I-O DATA GA-1280A graphic board.
    pub fn install_ga1280a(&mut self) {
        let ga1280a = Ga1280a::new();
        let first_vsync_cycle =
            self.current_cycle + ga1280a.display_period_cycles(self.clocks.cpu_clock_hz);
        self.ga1280a = Some(ga1280a);
        self.scheduler.schedule(Event98::GaVsync, first_vsync_cycle);
        self.update_next_event_cycle();
    }

    /// Composes and snapshots the GA-1280A framebuffer synchronously.
    pub fn ga1280a_present_now(&mut self) {
        if let Some(ga) = self.ga1280a.as_mut() {
            ga.on_vsync_start();
        }
        self.render_ga1280a_frame();
    }

    fn render_ga1280a_frame(&mut self) {
        let Some(ga) = self.ga1280a.as_ref() else {
            return;
        };
        let snapshot = ga.render_snapshot();
        let inputs = Self::ga1280a_render_inputs(snapshot);
        let (width, height) =
            compose_ga1280a(self.software_renderer.framebuffer_mut(), &inputs, true);
        self.display_width = width;
        self.display_height = height;
    }

    fn ga1280a_render_inputs(snapshot: Ga1280aRenderSnapshot<'_>) -> Ga1280aRenderInputs<'_> {
        let mode = match snapshot.plane_mode {
            device::ga1280a::Ga1280aPlaneMode::Indexed8 => Ga1280aRenderMode::Indexed8,
            device::ga1280a::Ga1280aPlaneMode::DirectColor16 => Ga1280aRenderMode::DirectColor16,
            device::ga1280a::Ga1280aPlaneMode::FullColor24 => Ga1280aRenderMode::FullColor24,
        };
        Ga1280aRenderInputs {
            mode,
            width: snapshot.width,
            height: snapshot.height,
            pixel_map_width: snapshot.pixel_map_width,
            pixel_map_height: snapshot.pixel_map_height,
            stride_bytes: snapshot.stride_bytes,
            display_offset_pixels: snapshot.display_offset_pixels,
            palette: snapshot.palette,
            visible_mask: snapshot.visible_mask,
            vram: snapshot.vram,
            cursor: Ga1280aCursorRenderInputs {
                visible: snapshot.cursor.visible,
                x: snapshot.cursor.x,
                y: snapshot.cursor.y,
                colors: snapshot.cursor.colors,
                xor_pattern: snapshot.cursor.xor_pattern,
                and_pattern: snapshot.cursor.and_pattern,
            },
        }
    }

    /// Returns the installed I-O DATA GA-1280A state, if any.
    pub fn ga1280a_state(&self) -> Option<&Ga1280aState> {
        self.ga1280a.as_ref().map(|ga| &ga.state)
    }

    /// Installs the PC-9801-26K sound board (YM2203 OPN).
    ///
    /// When `alternate_timers` is `true`, the board uses `FmTimer2A`/`FmTimer2B`
    /// event kinds instead of `FmTimerA`/`FmTimerB` (for dual-board configurations
    /// where the 86 board uses the primary timer events).
    pub fn install_soundboard_26k(&mut self, alternate_timers: bool) {
        let sample_rate = self.clocks.sample_rate;
        self.soundboard_26k = Some(Soundboard26k::new(
            self.clocks.cpu_clock_hz,
            sample_rate,
            alternate_timers,
        ));
        self.resolve_dual_soundboard_irq_conflict();
    }

    /// Installs the PC-9801-14 Music Generator board (TMS3631).
    pub fn install_soundboard_14(&mut self) {
        let sample_rate = self.clocks.sample_rate;
        self.soundboard_14 = Some(Soundboard14::new(self.clocks.cpu_clock_hz, sample_rate));
    }

    /// Installs the PC-9801-86 sound board (YM2608 OPNA + PCM86).
    ///
    /// `rhythm_rom` is the optional 8 KB `ym2608.rom` ADPCM-A rhythm ROM.
    /// `adpcm_ram` enables the 256 KiB ADPCM-B sample RAM upgrade.
    /// When installed, the 86 board replaces the 26K for FM/SSG ports
    /// and adds extended register and PCM86 ports.
    pub fn install_soundboard_86(&mut self, rhythm_rom: Option<&[u8]>, adpcm_ram: bool) {
        let sample_rate = self.clocks.sample_rate;
        self.soundboard_86 = Some(Soundboard86::new(
            self.clocks.cpu_clock_hz,
            sample_rate,
            rhythm_rom,
            adpcm_ram,
            self.machine_model,
        ));
        self.resolve_dual_soundboard_irq_conflict();
    }

    /// Installs a Creative Sound Blaster 16 (CT2720) sound board.
    ///
    /// The SB16 uses completely different I/O ports (base + 0x2000 range)
    /// and can coexist with the NEC 26K/86 boards.
    pub fn install_sound_blaster_16(&mut self) {
        let sample_rate = self.clocks.sample_rate;
        self.sound_blaster_16 = Some(SoundBlaster16::new(self.clocks.cpu_clock_hz, sample_rate));
    }

    /// Installs a Roland MT-32 sound module for MPU-PC98II MIDI output.
    #[cfg(feature = "mt32")]
    pub fn install_mt32(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::mt32::MuntError> {
        self.mt32 = Some(device::mt32::Mt32::new(rom_directory)?);
        Ok(())
    }

    /// Installs a Roland SC-55 sound module for MPU-PC98II MIDI output.
    #[cfg(feature = "sc55")]
    pub fn install_sc55(
        &mut self,
        rom_directory: &std::path::Path,
    ) -> Result<(), device::sc55::Sc55Error> {
        self.sc55 = Some(device::sc55::Sc55::new(rom_directory)?);
        Ok(())
    }

    fn resolve_dual_soundboard_irq_conflict(&mut self) {
        let (Some(soundboard_26k), Some(soundboard_86)) =
            (&mut self.soundboard_26k, &self.soundboard_86)
        else {
            return;
        };

        // 86+26K dual-board setups must not share the same IRQ line.
        // NP21W resolves the default 12/12 collision by moving the 26K
        // board to IRQ10.
        if soundboard_26k.state.irq_line == soundboard_86.state.irq_line {
            soundboard_26k.state.irq_line = if soundboard_26k.state.irq_line == 12 {
                10
            } else {
                12
            };
        }
    }

    /// Returns the CPU cycle at which the next scheduled event fires, if any.
    pub fn next_event_cycle(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Returns whether the PIC has a pending IRQ for the CPU.
    pub fn has_irq_pending(&self) -> bool {
        self.pic.has_pending_irq()
    }

    /// Returns PIC master chip debug info (IRR, IMR, ISR).
    pub fn pic_debug(&self) -> (u8, u8, u8) {
        let c = &self.pic.state.chips[0];
        (c.irr, c.imr, c.isr)
    }

    /// Returns PIT channel 0 debug info (ctrl, value, flag).
    pub fn pit_debug(&self) -> (u8, u16, u8) {
        let ch = &self.pit.state.channels[0];
        (ch.ctrl, ch.value, ch.flag)
    }

    /// Returns a reference to the beeper's saveable state (buzzer enable,
    /// pit reload, etc.).
    pub fn beeper_state(&self) -> &device::beeper::BeeperState {
        &self.beeper.state
    }

    /// Returns the beeper hardware architecture variant for this machine.
    pub fn beeper_kind(&self) -> common::BeeperKind {
        self.beeper.kind()
    }

    /// Returns the next scheduled event cycle (if any).
    pub fn next_event_debug(&self) -> Option<u64> {
        self.scheduler.next_event_cycle()
    }

    /// Returns a host-formatted overview of current HLE DOS memory usage.
    pub fn debug_memory_overview_lines(&mut self) -> Option<Vec<String>> {
        let access_page = self.access_page_index();
        let dos = self.dos.as_ref()?;
        let memory = dos_adapter::DosMemoryAccess::new(
            &mut self.memory,
            access_page,
            self.b_bank_ems,
            self.vram_ems_bank,
        );
        Some(dos.debug_memory_overview_lines(&memory))
    }

    fn update_plane_e_mapping(&mut self) {
        if self.pegc.is_256_color_active() {
            self.memory.set_e_plane_enabled(false);
        } else {
            self.memory.set_e_plane_enabled(
                self.graphics_extension_enabled && self.display_control.is_palette_analog_mode(),
            );
        }
    }

    fn mouse_timer_irq_enabled(&self) -> bool {
        (self.mouse_ppi.state.port_c & 0x10) == 0
    }

    fn mouse_timer_period_cycles(&self) -> u64 {
        let hz = match self.mouse_timer_setting & 0x03 {
            0x00 => 120u64,
            0x01 => 60u64,
            0x02 => 30u64,
            _ => 15u64,
        };
        let cpu = u64::from(self.clocks.cpu_clock_hz);
        cpu.div_ceil(hz)
    }

    fn schedule_mouse_timer(&mut self) {
        let next = self
            .current_cycle
            .wrapping_add(self.mouse_timer_period_cycles().max(1));
        self.scheduler.schedule(Event98::MouseTimer, next);
    }

    fn is_memory_switch_address(address: u32) -> bool {
        (0xA3FE2..=0xA3FFE).contains(&address) && (address - 0xA3FE2).is_multiple_of(4)
    }

    fn read_byte_with_access_page(&self, address: u32) -> u8 {
        if (0x80000..=0x9FFFF).contains(&address) && self.ram_window != 0x08 {
            let physical = ((self.ram_window & 0xFE) as u32) * 0x10000 + (address - 0x80000);
            if (0xE0000..=0xFFFFF).contains(&physical)
                && (self.memory.state.shadow_control & 0x04) != 0
            {
                return 0xFF;
            }
            return self.memory.read_byte(physical);
        }
        if let Some(ga) = self.ga1280a.as_ref()
            && let Some(value) = ga.window_read_byte(address)
        {
            return value;
        }
        match address {
            0xA4000..=0xA4FFF if self.grcg.state.chip >= 2 => {
                let window = self
                    .cgrom
                    .compute_window(self.display_control.is_font_8x16_mode());
                let line = ((address >> 1) & 0x0F) as usize;
                if address & 1 != 0 {
                    self.memory.font_read(window.high + line)
                } else {
                    self.memory.font_read(window.low + line)
                }
            }
            0xA8000..=0xAFFFF => {
                if self.pegc.is_256_color_active() {
                    if self.pegc.is_packed_pixel_mode() {
                        let vram = self.memory.state.pegc_vram.as_ref().unwrap().as_slice();
                        return self.pegc.packed_read_byte(0, address - 0xA8000, vram);
                    }
                    return 0;
                }
                self.memory.read_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                )
            }
            0xB0000..=0xBFFFF => {
                if self.pegc.is_256_color_active() && address <= 0xB7FFF {
                    if self.pegc.is_packed_pixel_mode() {
                        let vram = self.memory.state.pegc_vram.as_ref().unwrap().as_slice();
                        return self.pegc.packed_read_byte(1, address - 0xB0000, vram);
                    }
                    return 0;
                }
                self.memory.read_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                )
            }
            // 640KB FDD HLE ROM overlay (PC-9801-09-compatible expansion ROM area).
            0xD6000..=0xD6FFF => {
                if self.fdd640k_hle.rom_installed() && !self.memory.umb_region_enabled() {
                    self.fdd640k_hle.read_rom_byte((address - 0xD6000) as usize)
                } else {
                    self.memory.read_byte(address)
                }
            }
            // SASI HLE ROM overlay (expansion ROM area).
            0xD7000..=0xD7FFF => {
                if self.sasi.rom_installed() && !self.memory.umb_region_enabled() {
                    self.sasi.read_rom_byte((address - 0xD7000) as usize)
                } else {
                    self.memory.read_byte(address)
                }
            }
            // IDE HLE ROM overlay (expansion ROM area).
            0xD8000..=0xD9FFF => {
                if self.ide.rom_installed() && !self.memory.umb_region_enabled() {
                    self.ide.read_rom_byte((address - 0xD8000) as usize)
                } else {
                    self.memory.read_byte(address)
                }
            }
            0xE0000..=0xE7FFF => {
                if self.pegc.is_256_color_active() {
                    return self.pegc.mmio_read_byte(address - 0xE0000);
                }
                self.memory.read_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                )
            }
            _ => {
                if self.machine_model.has_pegc() {
                    let is_pegc_range = (0xF00000..=0xF7FFFF).contains(&address)
                        || (0xFFF00000..=0xFFF7FFFF).contains(&address);
                    if is_pegc_range {
                        if self.pegc.is_upper_vram_enabled() {
                            return self.memory.state.pegc_vram.as_ref().unwrap().as_slice()
                                [(address & 0x7FFFF) as usize];
                        }
                        return 0xFF;
                    }
                }
                self.memory.read_byte(address)
            }
        }
    }

    fn write_byte_with_access_page(&mut self, address: u32, value: u8) {
        if Self::is_memory_switch_address(address)
            && !self.display_control.is_memory_switch_write_enabled()
        {
            return;
        }

        if (0x80000..=0x9FFFF).contains(&address) && self.ram_window != 0x08 {
            let physical = ((self.ram_window & 0xFE) as u32) * 0x10000 + (address - 0x80000);
            if (0xE0000..=0xFFFFF).contains(&physical)
                && (self.memory.state.shadow_control & 0x04) != 0
            {
                return;
            }
            self.memory.write_byte(physical, value);
            return;
        }

        if let Some(ga) = self.ga1280a.as_mut()
            && ga.window_write_byte(address, value)
        {
            return;
        }

        match address {
            0xA4000..=0xA4FFF if self.grcg.state.chip >= 2 => {
                let window = self
                    .cgrom
                    .compute_window(self.display_control.is_font_8x16_mode());
                if (address & 1 != 0) && window.writable {
                    let line = ((address >> 1) & 0x0F) as usize;
                    self.memory.font_write(window.high + line, value);
                }
            }
            0xA8000..=0xAFFFF => {
                if self.pegc.is_256_color_active() {
                    if self.pegc.is_packed_pixel_mode() {
                        let vram = self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice();
                        self.pegc
                            .packed_write_byte(0, address - 0xA8000, value, vram);
                    }
                    return;
                }
                self.memory.write_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                    value,
                );
            }
            0xB0000..=0xBFFFF => {
                if self.pegc.is_256_color_active() && address <= 0xB7FFF {
                    if self.pegc.is_packed_pixel_mode() {
                        let vram = self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice();
                        self.pegc
                            .packed_write_byte(1, address - 0xB0000, value, vram);
                    }
                    return;
                }
                self.memory.write_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                    value,
                );
            }
            0xE0000..=0xE7FFF => {
                if self.pegc.is_256_color_active() {
                    self.pegc.mmio_write_byte(address - 0xE0000, value);
                    return;
                }
                self.memory.write_byte_with_access_page(
                    address,
                    self.access_page_index(),
                    self.b_bank_ems,
                    self.vram_ems_bank,
                    value,
                );
            }
            _ => {
                if self.machine_model.has_pegc() {
                    let is_pegc_range = (0xF00000..=0xF7FFFF).contains(&address)
                        || (0xFFF00000..=0xFFF7FFFF).contains(&address);
                    if is_pegc_range {
                        if self.pegc.is_upper_vram_enabled() {
                            self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice()
                                [(address & 0x7FFFF) as usize] = value;
                        }
                        return;
                    }
                }
                self.memory.write_byte(address, value);
            }
        }
    }

    /// Returns the composed RGBA framebuffer rendered at the last VSYNC.
    ///
    /// The returned slice covers the full backing buffer; only the top-left
    /// `display_dimensions()` region holds valid pixels for the latest frame.
    pub fn display_framebuffer(&self) -> &[u8] {
        self.software_renderer.framebuffer()
    }

    /// Returns the `(width, height)` of the valid region in
    /// [`display_framebuffer`](Self::display_framebuffer).
    pub fn display_dimensions(&self) -> (u32, u32) {
        (self.display_width, self.display_height)
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

    /// Returns whether the GA-1280A board is currently driving the monitor.
    pub fn ga1280a_is_driving_monitor(&self) -> bool {
        self.ga1280a
            .as_ref()
            .is_some_and(|ga| ga.is_driving_monitor())
    }

    /// Composes the current display state into the renderer's internal
    /// framebuffer. Called at every VSYNC.
    pub fn render_display_frame(&mut self) {
        if self.memory.take_font_rom_dirty() {
            self.software_renderer
                .update_font_rom(self.memory.font_rom_data());
        }

        // Blink timing: derive a phase counter from the monotonic VSYNC blink_counter.
        //   threshold = cursor_blink_rate * 2, or 64 when rate == 0
        //   count increments every `threshold` VSYNCs
        //   cursor: count & 1 (50% duty), text: (count & 3) != 0 (75/25% duty)
        let blink_rate = u16::from(self.gdc_master.state.cursor_blink_rate);
        let blink_threshold = if blink_rate == 0 {
            64u16
        } else {
            blink_rate * 2
        };
        let blink_count = self.gdc_master.state.blink_counter / blink_threshold;
        let text_blink_visible = (blink_count & 3) != 0;

        let video_mode = self.display_control.state.video_mode;
        let is_16_color = self.display_control.is_16_color();
        let text_enabled = self.gdc_master.state.display_enabled;
        let graphics_enabled = self.gdc_slave.state.display_enabled;
        let global_enabled = self.display_control.is_display_enabled_global();
        let is_graphics_monochrome = self.display_control.is_graphics_monochrome();
        let is_palette_analog_mode = self.display_control.is_palette_analog_mode();
        let is_kac_dot_access_mode = self.display_control.is_kac_dot_access_mode();
        let interlace_on = self.gdc_slave.state.interlace_mode == 0x09;

        let mut palette_rgba = [0u32; 16];
        if is_palette_analog_mode {
            for (i, slot) in palette_rgba.iter_mut().enumerate() {
                let [g4, r4, b4] = self.palette.state.analog[i];
                let red = (r4 & 0x0F) * 17;
                let green = (g4 & 0x0F) * 17;
                let blue = (b4 & 0x0F) * 17;
                *slot = pack_rgba(red, green, blue);
            }
        } else {
            let (fixed, digital) = palette_rgba.split_at_mut(DIGITAL_GRAPHICS_PALETTE_BASE);
            for (i, slot) in fixed.iter_mut().enumerate() {
                *slot = pack_fixed_color(i as u8);
            }
            for (i, slot) in digital.iter_mut().enumerate() {
                *slot = pack_digital_graphics_color(&self.palette.state.digital, i);
            }
        }

        let gdc_text_pitch = u32::from(self.gdc_master.state.pitch);

        let mut gdc_scroll_start_line = [0u32; 4];
        for (i, slot) in gdc_scroll_start_line.iter_mut().enumerate() {
            let area = &self.gdc_master.state.scroll[i];
            *slot = area.start_address | (u32::from(area.line_count) << 16);
        }

        // In 2.5 MHz mode (mode2 bits 9-10 clear): pitch is in words, multiply by 2.
        // In 5 MHz mode (mode2 bits 9-10 set):     pitch is already in bytes.
        //
        // PEGC packed-pixel mode is special: the display engine bypasses the
        // µPD7220's planar/word interpretation and addresses VRAM as a flat
        // byte stream (1 byte per pixel). The pitch the renderer feeds to the
        // PEGC scanline iterator is then `slave.pitch` taken verbatim - the
        // 2.5 MHz word doubling does not apply.
        let gdc_5mhz = self.display_control.is_gdc_5mhz();
        let raw_pitch = self.gdc_slave.state.pitch;
        let graphics_pitch = if gdc_5mhz || self.pegc.is_256_color_active() {
            raw_pitch
        } else {
            raw_pitch * 2
        };
        let gdc_graphics_pitch = u32::from(graphics_pitch & 0xFE);

        let graphics_monochrome_mask = if is_graphics_monochrome {
            if is_palette_analog_mode {
                let mut mask: u32 = 0;
                for i in 0..16u32 {
                    if self.palette.state.analog[i as usize][0] & 0x08 != 0 {
                        mask |= 1 << i;
                    }
                }
                mask
            } else {
                digital_monochrome_mask(&self.palette.state.digital)
            }
        } else {
            0
        };

        let gdc_graphics_lines_per_row = u32::from(self.gdc_slave.state.lines_per_row);
        let gdc_graphics_zoom_display = u32::from(self.gdc_slave.state.zoom_display);

        // Graphics scroll partitions - double partition line counts for interlace ON mode.
        let mut gdc_graphics_scroll = [0u32; 4];
        for (i, slot) in gdc_graphics_scroll.iter_mut().enumerate() {
            let area = &self.gdc_slave.state.scroll[i];
            let line_count = if interlace_on {
                area.line_count.saturating_mul(2)
            } else {
                area.line_count
            };
            *slot = area.start_address | (u32::from(line_count) << 16);
        }

        let kanji_high_mask: u8 = if is_kac_dot_access_mode { 0x00 } else { 0xFF };

        let crtc_pl_bl =
            u32::from(self.crtc.state.regs[0]) | (u32::from(self.crtc.state.regs[1]) << 16);
        let crtc_cl_ssl =
            u32::from(self.crtc.state.regs[2]) | (u32::from(self.crtc.state.regs[3]) << 16);
        let crtc_sur_sdr =
            u32::from(self.crtc.state.regs[4]) | (u32::from(self.crtc.state.regs[5]) << 16);

        let cursor_blink_visible = if self.gdc_master.state.cursor_blink {
            true
        } else {
            (blink_count & 1) != 0
        };
        let cursor_enabled = self.gdc_master.state.cursor_display && cursor_blink_visible;
        let cursor_addr = self.gdc_master.state.ead;
        let cursor_top = u32::from(self.gdc_master.state.cursor_top & 0x1F);
        let cursor_bottom = u32::from(self.gdc_master.state.cursor_bottom & 0x1F);

        let gdc_graphics_al = u32::from(self.gdc_slave.state.al);

        let graphics = match self.pegc.is_256_color_active() {
            false => {
                let display_page_base = self.display_page_index() * GRAPHICS_PAGE_SIZE_BYTES;
                let e_page_base = self.display_page_index() * E_PLANE_PAGE_SIZE_BYTES;
                GraphicsInput::Gdc(GdcGraphicsInput {
                    b_plane: &self.memory.state.graphics_vram
                        [display_page_base..display_page_base + 0x8000],
                    r_plane: &self.memory.state.graphics_vram
                        [display_page_base + 0x8000..display_page_base + 0x10000],
                    g_plane: &self.memory.state.graphics_vram
                        [display_page_base + 0x10000..display_page_base + 0x18000],
                    e_plane: &self.memory.state.e_plane_vram
                        [e_page_base..e_page_base + E_PLANE_PAGE_SIZE_BYTES],
                    lines_per_row: gdc_graphics_lines_per_row,
                    zoom_display: gdc_graphics_zoom_display,
                    monochrome_mask: graphics_monochrome_mask,
                    is_16_color,
                })
            }
            true => {
                let is_packed = self.pegc.is_packed_pixel_mode();
                let is_one_screen =
                    self.pegc.state.screen_mode == device::pegc::PegcScreenMode::OneScreen;
                let display_page = self.display_page_index() as u32;

                let mut palette_rgba_256 = [0u32; 256];
                for (i, slot) in palette_rgba_256.iter_mut().enumerate() {
                    let [green, red, blue] = self.pegc.state.palette_256[i];
                    *slot = u32::from(red)
                        | (u32::from(green) << 8)
                        | (u32::from(blue) << 16)
                        | 0xFF00_0000;
                }

                let pegc_flags =
                    u32::from(is_packed) | (u32::from(is_one_screen) << 1) | (display_page << 2);

                let vram: &[u8] = self
                    .memory
                    .state
                    .pegc_vram
                    .as_ref()
                    .map(|v| v.as_ref().as_ref())
                    .unwrap_or(&[]);

                GraphicsInput::Pegc(Box::new(PegcRenderInputs {
                    palette_rgba_256,
                    pegc_flags,
                    vram,
                }))
            }
        };

        let inputs = RenderInputs {
            text_vram: &self.memory.state.text_vram,
            gdc_text_pitch,
            gdc_scroll_start_line,
            video_mode: u32::from(video_mode),
            crtc_pl_bl,
            crtc_cl_ssl,
            crtc_sur_sdr,
            kanji_high_mask,
            attr_semigraphics_mode: self.display_control.is_attr_semigraphics_enabled(),
            fontsel_8x16: self.display_control.is_font_8x16_mode(),
            blink_visible: text_blink_visible,
            cursor_visible: cursor_enabled,
            cursor_addr,
            cursor_top,
            cursor_bottom,
            gdc_graphics_pitch,
            gdc_graphics_scroll,
            gdc_graphics_display_mode_is_graphics: self.gdc_slave.state.display_mode
                == DISPLAY_MODE_GRAPHICS,
            gdc_graphics_al,
            crt_31khz_enabled: self.display_control.is_crt_31khz_enabled(),
            palette_rgba,
            global_enabled,
            text_enabled,
            graphics_enabled,
            graphics,
        };

        self.display_width = SoftwareRenderer::WIDTH as u32;
        self.display_height = SoftwareRenderer::native_height(&inputs);

        self.software_renderer.render(&inputs);
    }

    /// Generates audio samples for the current frame.
    ///
    /// Mixes beeper (PIT ch1 square wave) with YM2203 FM + SSG output.
    pub fn generate_audio_samples(&mut self, volume: f32, output: &mut [f32]) -> usize {
        let beeper_count = self.beeper.generate_samples(
            self.current_cycle,
            self.clocks.cpu_clock_hz,
            self.clocks.pit_clock_hz,
            self.clocks.sample_rate,
            volume,
            output,
        );

        if let Some(ref mut sb86) = self.soundboard_86 {
            sb86.generate_samples(self.current_cycle, self.clocks.cpu_clock_hz, volume, output);
        }
        self.process_soundboard_86_actions();
        if let Some(ref mut sb26k) = self.soundboard_26k {
            sb26k.generate_samples(self.current_cycle, self.clocks.cpu_clock_hz, volume, output);
        }
        if let Some(ref mut sb14) = self.soundboard_14 {
            sb14.generate_samples(volume, output);
        }
        if let Some(ref mut sb16) = self.sound_blaster_16 {
            sb16.generate_samples(self.current_cycle, self.clocks.cpu_clock_hz, volume, output);
        }
        self.process_soundboard_sb16_actions();

        self.ide.generate_cd_audio_samples(volume, output);

        #[cfg(feature = "mt32")]
        if let Some(ref mt32) = self.mt32 {
            mt32.exchange(volume, output, |buf| self.mpu401.flush_midi_into(buf));
        }

        #[cfg(feature = "sc55")]
        if let Some(ref sc55) = self.sc55 {
            sc55.exchange(volume, output, |buf| self.mpu401.flush_midi_into(buf));
        }

        beeper_count
    }

    /// Reads a single byte directly from the full address space without side effects.
    pub fn read_byte_direct(&self, physical_address: u32) -> u8 {
        self.read_byte_with_access_page(physical_address)
    }

    /// Sets the master-GDC text cursor position to (row, col).
    ///
    /// Intended for test harnesses that need to position the text cursor
    /// without going through INT 18H AH=13h. Writes only the GDC execute
    /// address (ead) - callers that also need the HLE DOS IOSYS fields
    /// updated must do that separately.
    pub fn set_text_cursor_position(&mut self, row: u8, col: u8) {
        self.gdc_master.state.ead = u32::from(row) * 80 + u32::from(col);
    }

    /// Returns a reference to the raw text VRAM contents (16 KB).
    pub fn text_vram(&self) -> &[u8] {
        self.memory.state.text_vram.as_slice()
    }

    /// Returns a reference to the raw graphics VRAM (B/R/G planes, 2 pages).
    pub fn graphics_vram(&self) -> &[u8] {
        self.memory.state.graphics_vram.as_slice()
    }

    /// Returns a reference to the E-plane VRAM (2 pages).
    pub fn e_plane_vram(&self) -> &[u8] {
        self.memory.state.e_plane_vram.as_slice()
    }

    /// Returns the kanji font ROM data (512 KB, double-byte 16×16 glyphs).
    pub fn font_rom_data(&self) -> &[u8] {
        self.memory.font_rom_data()
    }

    /// Returns `true` if gaiji were written since the last call, and clears the flag.
    pub fn take_font_rom_dirty(&mut self) -> bool {
        self.memory.take_font_rom_dirty()
    }

    fn a20_mask(&self, address: u32) -> u32 {
        if self.a20_enabled {
            address
        } else {
            address & !0x0010_0000
        }
    }

    pub(crate) fn save_state(&self, cpu: crate::Pc98CpuState) -> crate::Pc98MachineState {
        crate::Pc98MachineState {
            cpu,
            machine_model: self.machine_model,
            memory: self.memory.state.clone(),
            clocks: self.clocks,
            pic: self.pic.state.clone(),
            scheduler: self.scheduler.state.clone(),
            pit: self.pit.state.clone(),
            gdc_master: self.gdc_master.state.clone(),
            gdc_slave: self.gdc_slave.state.clone(),
            current_cycle: self.current_cycle,
            next_event_cycle: self.next_event_cycle,
            nmi_enabled: self.nmi_enabled,
            keyboard: self.keyboard.state.clone(),
            serial: self.serial.state.clone(),
            a20_enabled: self.a20_enabled,
            fdc_1mb: self.floppy.fdc_1mb().state.clone(),
            fdc_640k: self.floppy.fdc_640k().state.clone(),
            fdc_media: self.floppy.fdc_media(),
            fdd320_ppi: self.fdd320_ppi.state.clone(),
            vram_ems_bank: self.vram_ems_bank,
            ram_window: self.ram_window,
            system_ppi: self.system_ppi.state.clone(),
            printer: self.printer.state.clone(),
            cgrom: self.cgrom.state.clone(),
            grcg: self.grcg.state.clone(),
            egc: self.egc.state.clone(),
            display_control: self.display_control.state.clone(),
            crtc: self.crtc.state.clone(),
            palette: self.palette.state.clone(),
            soundboard_14: self.soundboard_14.as_ref().map(|sb| sb.save_state()),
            soundboard_26k: self.soundboard_26k.as_ref().map(|sb| sb.save_state()),
            soundboard_86: self.soundboard_86.as_ref().map(|sb| sb.save_state()),
            sound_blaster_16: self.sound_blaster_16.as_ref().map(|sb| sb.save_state()),
            ga1280a: self.ga1280a.as_ref().map(|ga| ga.state.clone()),
            beeper: self.beeper.state.clone(),
            mouse_ppi: self.mouse_ppi.state.clone(),
            mouse_timer_setting: self.mouse_timer_setting,
            hole_15m_control: self.hole_15m_control,
            protected_memory_max: self.protected_memory_max,
            b_bank_ems: self.b_bank_ems,
            tram_wait: self.tram_wait,
            vram_wait: self.vram_wait,
            grcg_wait: self.grcg_wait,
            bios_interval_timer_active: self.bios_interval_timer_active,
        }
    }

    pub(crate) fn load_peripherals(&mut self, state: &crate::Pc98MachineState) {
        self.machine_model = state.machine_model;
        self.memory.state = state.memory.clone();
        self.pic.state = state.pic.clone();
        self.pic.invalidate_irq_cache();
        self.scheduler.state = state.scheduler.clone();
        self.current_cycle = state.current_cycle;
        self.next_event_cycle = state.next_event_cycle;
        self.nmi_enabled = state.nmi_enabled;
        self.clocks = state.clocks;
        self.pit.state = state.pit.clone();
        self.gdc_master.state = state.gdc_master.clone();
        self.gdc_slave.state = state.gdc_slave.clone();
        self.keyboard.state = state.keyboard.clone();
        self.keyboard_chained_raw_code = None;
        self.serial.state = state.serial.clone();
        self.a20_enabled = state.a20_enabled;
        self.floppy.fdc_1mb_mut().state = state.fdc_1mb.clone();
        self.floppy.fdc_640k_mut().state = state.fdc_640k.clone();
        self.floppy.set_fdc_media(state.fdc_media);
        self.fdd320_ppi.state = state.fdd320_ppi.clone();
        self.vram_ems_bank = state.vram_ems_bank;
        self.ram_window = state.ram_window;
        self.system_ppi.state = state.system_ppi.clone();
        self.printer.state = state.printer.clone();
        self.cgrom.state = state.cgrom.clone();
        self.grcg.state = state.grcg.clone();
        self.egc.state = state.egc.clone();
        self.display_control.state = state.display_control.clone();
        self.crtc.state = state.crtc.clone();
        self.palette.state = state.palette.clone();
        if let (Some(sb14), Some(saved)) = (&mut self.soundboard_14, &state.soundboard_14) {
            sb14.load_state(
                saved,
                self.clocks.cpu_clock_hz,
                state.clocks.sample_rate,
                state.current_cycle,
            );
        }
        if let (Some(sb26k), Some(saved)) = (&mut self.soundboard_26k, &state.soundboard_26k) {
            sb26k.load_state(
                saved,
                self.clocks.cpu_clock_hz,
                state.clocks.sample_rate,
                state.current_cycle,
            );
        }
        if let (Some(sb86), Some(saved)) = (&mut self.soundboard_86, &state.soundboard_86) {
            sb86.load_state(
                saved,
                self.clocks.cpu_clock_hz,
                state.clocks.sample_rate,
                state.current_cycle,
                None,
            );
        }
        if let (Some(sb16), Some(saved)) = (&mut self.sound_blaster_16, &state.sound_blaster_16) {
            sb16.load_state(
                saved,
                self.clocks.cpu_clock_hz,
                state.clocks.sample_rate,
                state.current_cycle,
            );
        }
        self.ga1280a = state.ga1280a.clone().map(Ga1280a::from_state);
        self.render_ga1280a_frame();
        self.beeper.state = state.beeper.clone();
        self.mouse_ppi.state = state.mouse_ppi.clone();
        self.mouse_ppi.set_cpu_clock(self.clocks.cpu_clock_hz);
        self.mouse_timer_setting = state.mouse_timer_setting;
        self.hole_15m_control = state.hole_15m_control;
        self.protected_memory_max = state.protected_memory_max;
        self.b_bank_ems = state.b_bank_ems;
        self.tram_wait = state.tram_wait;
        self.vram_wait = state.vram_wait;
        self.grcg_wait = state.grcg_wait;
        self.bios_interval_timer_active = state.bios_interval_timer_active;
        self.reset_pending = false;
        self.shutdown_requested = false;
    }

    fn process_soundboard_86_actions(&mut self) {
        if let Some(ref mut sb86) = self.soundboard_86 {
            let pcm86_pending =
                self.scheduler.state.fire_cycles[Event98::Pcm86Irq as usize].is_some();
            for action in sb86.drain_actions(pcm86_pending) {
                match *action {
                    Soundboard86Action::ScheduleTimer { kind, fire_cycle } => {
                        self.scheduler.schedule(kind.into(), fire_cycle);
                    }
                    Soundboard86Action::CancelTimer { kind } => {
                        self.scheduler.cancel(kind.into());
                    }
                    Soundboard86Action::AssertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            true,
                        );
                    }
                    Soundboard86Action::DeassertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            false,
                        );
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    fn process_soundboard_14_actions(&mut self) {
        if let Some(ref mut sb14) = self.soundboard_14 {
            for action in sb14.drain_actions() {
                match *action {
                    Soundboard14Action::ScheduleTimer { kind, fire_cycle } => {
                        self.scheduler.schedule(kind.into(), fire_cycle);
                    }
                    Soundboard14Action::CancelTimer { kind } => {
                        self.scheduler.cancel(kind.into());
                    }
                    Soundboard14Action::AssertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            true,
                        );
                    }
                    Soundboard14Action::DeassertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            false,
                        );
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    fn process_soundboard_actions(&mut self) {
        if let Some(ref mut sb26k) = self.soundboard_26k {
            for action in sb26k.drain_actions() {
                match *action {
                    Soundboard26kAction::ScheduleTimer { kind, fire_cycle } => {
                        self.scheduler.schedule(kind.into(), fire_cycle);
                    }
                    Soundboard26kAction::CancelTimer { kind } => {
                        self.scheduler.cancel(kind.into());
                    }
                    Soundboard26kAction::AssertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            true,
                        );
                    }
                    Soundboard26kAction::DeassertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            false,
                        );
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    fn process_soundboard_sb16_actions(&mut self) {
        if let Some(ref mut sb16) = self.sound_blaster_16 {
            let dsp_sample_rate = sb16.state.dsp.sample_rate;
            let dsp_dma_format = sb16.state.dsp.dma_format;
            let dsp_dma_channel = sb16.state.dsp.dma_channel as usize;
            let dsp_dma_channel_register = sb16.state.dsp.dma_channel_register;
            for action in sb16.drain_actions() {
                match *action {
                    SoundboardSb16Action::ScheduleTimer { kind, fire_cycle } => {
                        self.scheduler.schedule(kind.into(), fire_cycle);
                    }
                    SoundboardSb16Action::CancelTimer { kind } => {
                        self.scheduler.cancel(kind.into());
                    }
                    SoundboardSb16Action::AssertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            true,
                        );
                    }
                    SoundboardSb16Action::DeassertIrq { irq } => {
                        update_pic_irq(
                            &mut self.pic,
                            &mut self.tracer,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                            irq,
                            false,
                        );
                    }
                    SoundboardSb16Action::StartDma { channel: _ } => {
                        // When high-DMA channels are configured and the
                        // transfer is 16-bit, the software may have
                        // programmed the DMA controller with ISA-style word
                        // count and word address. Convert to byte-based
                        // values for the PC-98 8-bit DMA controller.
                        let high_dma_16bit = dsp_dma_channel_register & 0xE0 != 0
                            && device::sound_blaster_16::dma_format_is_16bit(dsp_dma_format);
                        if high_dma_16bit {
                            let ch = &mut self.dma.state.channels[dsp_dma_channel];
                            // Word count -> byte count.
                            let byte_count = (u32::from(ch.start_count) + 1) * 2 - 1;
                            ch.start_count = byte_count as u16;
                            ch.count = ch.start_count;
                            // Word address -> byte address (within 64K page).
                            let byte_address = ch.start_address.wrapping_shl(1);
                            ch.start_address = byte_address;
                            ch.address = byte_address;
                        }

                        Self::schedule_sb16_dma_from_params(
                            &mut self.scheduler,
                            dsp_sample_rate,
                            dsp_dma_format,
                            self.current_cycle,
                            self.current_cycle,
                            self.clocks.cpu_clock_hz,
                        );
                    }
                    SoundboardSb16Action::StopDma => {
                        self.scheduler.cancel(Event98::Sb16DspDma);
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    fn schedule_sb16_dma(
        scheduler: &mut Pc98Scheduler,
        sb16: &SoundBlaster16<SB16_PLATFORM_PC98>,
        reference_cycle: u64,
        current_cycle: u64,
        cpu_clock_hz: u32,
    ) {
        Self::schedule_sb16_dma_from_params(
            scheduler,
            sb16.state.dsp.sample_rate,
            sb16.state.dsp.dma_format,
            reference_cycle,
            current_cycle,
            cpu_clock_hz,
        );
    }

    fn schedule_sb16_dma_from_params(
        scheduler: &mut Pc98Scheduler,
        sample_rate: u32,
        dma_format: u8,
        reference_cycle: u64,
        current_cycle: u64,
        cpu_clock_hz: u32,
    ) {
        let sample_rate = sample_rate.max(1) as u64;
        let bytes_per_sample =
            device::sound_blaster_16::dma_format_bytes_per_sample(dma_format) as u64;
        let byte_rate = sample_rate * bytes_per_sample.max(1);
        let interval_cycles =
            device::sound_blaster_16::DMA_BATCH_SIZE as u64 * cpu_clock_hz as u64 / byte_rate;
        let fire_cycle = (reference_cycle + interval_cycles.max(1)).max(current_cycle + 1);
        scheduler.schedule(Event98::Sb16DspDma, fire_cycle);
    }

    fn handle_mpu_timer(&mut self) {
        let reschedule = self.mpu401.tick();
        if self.mpu401.take_irq() {
            self.raise_pic_irq(MPU_IRQ_LINE);
        }
        if reschedule {
            let step_cycles = self.mpu401.step_clock_cycles(self.clocks.cpu_clock_hz);
            self.scheduler
                .schedule(Event98::MpuTimer, self.current_cycle + step_cycles);
        }
    }

    fn sync_mpu_irq_and_timer(&mut self) {
        if self.mpu401.take_irq() {
            self.raise_pic_irq(MPU_IRQ_LINE);
        } else {
            self.clear_pic_irq(MPU_IRQ_LINE);
        }
        if self.mpu401.timer_active()
            && self.scheduler.state.fire_cycles[Event98::MpuTimer as usize].is_none()
        {
            let step_cycles = self.mpu401.step_clock_cycles(self.clocks.cpu_clock_hz);
            self.scheduler
                .schedule(Event98::MpuTimer, self.current_cycle + step_cycles);
        }
        if !self.mpu401.timer_active() {
            self.scheduler.cancel(Event98::MpuTimer);
        }
    }

    fn handle_sb16_dma_transfer(&mut self, event_fire_cycle: u64) {
        let (channel, batch_size, dma_active, is_recording, dma_format) = {
            let Some(ref sb16) = self.sound_blaster_16 else {
                return;
            };
            if !sb16.dma_transfer_pending() {
                return;
            }
            (
                sb16.state.dsp.dma_channel as usize,
                device::sound_blaster_16::DMA_BATCH_SIZE,
                sb16.state.dsp.dma_active,
                sb16.state.dsp.dma_is_recording,
                sb16.state.dsp.dma_format,
            )
        };

        if !dma_active {
            return;
        }

        let mask_20bit = self.dma_access_ctrl & 0x04 != 0;

        if is_recording {
            // Recording: generate silence and write to memory via DMA.
            let silence_byte = if device::sound_blaster_16::dma_format_is_16bit(dma_format) {
                0x00u8
            } else {
                0x80u8
            };
            let silence = [silence_byte; device::sound_blaster_16::DMA_BATCH_SIZE];
            let result = self
                .dma
                .transfer_write_to_memory(channel, &silence[..batch_size]);

            for &(addr, value) in &result.writes {
                let addr = if mask_20bit { addr & 0xF_FFFF } else { addr };
                self.write_byte_with_access_page(addr, value);
            }

            if let Some(ref mut sb16) = self.sound_blaster_16 {
                sb16.advance_dma_recording(result.writes.len() as u32);
                if result.terminal_count {
                    sb16.dma_terminal_count();
                }
            }
        } else {
            // Playback: read from memory via DMA.
            let result = self.dma.transfer_read_from_memory(channel, batch_size);

            let mut data: StackVec<u8, { device::sound_blaster_16::DMA_BATCH_SIZE }> =
                StackVec::new();
            for &addr in &result.addresses {
                let addr = if mask_20bit { addr & 0xF_FFFF } else { addr };
                data.push(self.read_byte_with_access_page(addr));
            }

            if let Some(ref mut sb16) = self.sound_blaster_16 {
                sb16.accept_dma_data(&data);
                if result.terminal_count {
                    sb16.dma_terminal_count();
                }
            }
        }

        self.process_soundboard_sb16_actions();

        // Reschedule relative to the original event fire cycle to prevent drift.
        if let Some(ref sb16) = self.sound_blaster_16
            && sb16.dma_transfer_pending()
        {
            Self::schedule_sb16_dma(
                &mut self.scheduler,
                sb16,
                event_fire_cycle,
                self.current_cycle,
                self.clocks.cpu_clock_hz,
            );
        }
    }

    fn update_next_event_cycle(&mut self) {
        self.next_event_cycle = self.scheduler.next_event_cycle().unwrap_or(u64::MAX);
    }

    fn process_events(&mut self) {
        let events = self.scheduler.pop_due_events(self.current_cycle);

        for event in &events {
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
                Event98::PitTimer0 => {
                    let raise_irq = self.pit.advance_timer0(self.current_cycle);
                    let cpu_cycles = self
                        .pit
                        .timer0_period_cycles(self.clocks.cpu_clock_hz, self.clocks.pit_clock_hz);
                    self.scheduler
                        .schedule(Event98::PitTimer0, self.current_cycle + cpu_cycles);
                    if raise_irq {
                        if self.current_cpu_protected_mode {
                            self.handle_bios_interval_timer_tick();
                        }
                        self.raise_pic_irq(0);
                    }
                }
                Event98::GdcVsync => {
                    // GA-1280A and GDC/PEGC share one framebuffer; only the
                    // side that drives the monitor may compose into it.
                    if !self.ga1280a_is_driving_monitor() {
                        self.render_display_frame();
                        self.trace_presentation();
                    }
                    self.tram_wait = 1;
                    self.vram_wait = 1;
                    self.grcg_wait = 1;
                    self.gdc_master.on_vsync_event();
                    self.gdc_slave.set_vsync(true);
                    if self.display_control.state.vsync_irq_enabled {
                        self.display_control.state.vsync_irq_enabled = false;
                        self.raise_pic_irq(2);
                    }
                    self.scheduler.schedule(
                        Event98::GdcDisplayStart,
                        self.current_cycle + self.gdc_master.state.vsync_blanking_period,
                    );
                }
                Event98::GdcDisplayStart => {
                    self.tram_wait = TRAM_WAIT_CYCLES;
                    self.vram_wait = VRAM_WAIT_CYCLES;
                    self.grcg_wait = GRCG_WAIT_CYCLES;
                    self.gdc_master.set_vsync(false);
                    self.gdc_slave.set_vsync(false);
                    self.scheduler.schedule(
                        Event98::GdcVsync,
                        self.current_cycle + self.gdc_master.state.display_period,
                    );
                }
                Event98::FdcExecution => {
                    self.handle_fdc_execution();
                }
                Event98::FdcInterrupt => {
                    self.handle_fdc_interrupt();
                }
                Event98::GdcDrawingComplete => {
                    self.gdc_slave.on_drawing_complete();
                }
                Event98::MouseTimer => {
                    if self.mouse_timer_irq_enabled() {
                        self.raise_pic_irq(MOUSE_TIMER_IRQ_LINE);
                        self.schedule_mouse_timer();
                    }
                }
                Event98::FmTimerA => {
                    if let Some(ref mut sb86) = self.soundboard_86 {
                        sb86.timer_expired(0, self.current_cycle);
                        self.process_soundboard_86_actions();
                    } else if let Some(ref mut sb26k) = self.soundboard_26k {
                        sb26k.timer_expired(0, self.current_cycle);
                        self.process_soundboard_actions();
                    }
                }
                Event98::FmTimerB => {
                    if let Some(ref mut sb86) = self.soundboard_86 {
                        sb86.timer_expired(1, self.current_cycle);
                        self.process_soundboard_86_actions();
                    } else if let Some(ref mut sb26k) = self.soundboard_26k {
                        sb26k.timer_expired(1, self.current_cycle);
                        self.process_soundboard_actions();
                    }
                }
                Event98::FmTimer2A => {
                    if let Some(ref mut sb26k) = self.soundboard_26k {
                        sb26k.timer_expired(0, self.current_cycle);
                        self.process_soundboard_actions();
                    }
                }
                Event98::FmTimer2B => {
                    if let Some(ref mut sb26k) = self.soundboard_26k {
                        sb26k.timer_expired(1, self.current_cycle);
                        self.process_soundboard_actions();
                    }
                }
                Event98::SasiExecution => {
                    self.handle_sasi_execution();
                }
                Event98::SasiInterrupt => {
                    self.handle_sasi_interrupt();
                }
                Event98::IdeExecution => {
                    self.handle_ide_execution();
                }
                Event98::IdeInterrupt => {
                    self.handle_ide_interrupt();
                }
                Event98::Pcm86Irq => {
                    if let Some(ref mut sb86) = self.soundboard_86 {
                        sb86.pcm86_timer_expired(self.current_cycle, self.clocks.cpu_clock_hz);
                        self.process_soundboard_86_actions();
                    }
                }
                Event98::Sb16OplTimerA => {
                    if let Some(ref mut sb16) = self.sound_blaster_16 {
                        sb16.timer_expired(0, self.current_cycle);
                        self.process_soundboard_sb16_actions();
                    }
                }
                Event98::Sb16OplTimerB => {
                    if let Some(ref mut sb16) = self.sound_blaster_16 {
                        sb16.timer_expired(1, self.current_cycle);
                        self.process_soundboard_sb16_actions();
                    }
                }
                Event98::Sb16DspDma => {
                    self.handle_sb16_dma_transfer(event.fire_cycle);
                }
                Event98::MpuTimer => {
                    self.handle_mpu_timer();
                }
                Event98::MusicGen14Timer => {
                    if let Some(ref mut sb14) = self.soundboard_14 {
                        sb14.timer_expired(self.current_cycle);
                        self.process_soundboard_14_actions();
                    }
                }
                Event98::GaVsync => {
                    if let Some(ga) = self.ga1280a.as_mut() {
                        ga.on_vsync_start();
                        let blanking = ga.blanking_period_cycles(self.clocks.cpu_clock_hz);
                        self.scheduler
                            .schedule(Event98::GaDisplayStart, self.current_cycle + blanking);
                    }
                    // GA-1280A and GDC/PEGC share one framebuffer; only the
                    // side that drives the monitor may compose into it.
                    if self.ga1280a_is_driving_monitor() {
                        self.render_ga1280a_frame();
                        self.trace_presentation();
                    }
                }
                Event98::GaDisplayStart => {
                    if let Some(ga) = self.ga1280a.as_mut() {
                        ga.on_display_start();
                        let display = ga.display_period_cycles(self.clocks.cpu_clock_hz);
                        self.scheduler
                            .schedule(Event98::GaVsync, self.current_cycle + display);
                    }
                }
            }
        }
        self.update_next_event_cycle();
    }

    fn gdc_address_to_plane_and_byte_offset(address: u32) -> (usize, usize) {
        let plane = (((address >> 14) & 0x03).wrapping_add(3) % 4) as usize;
        let byte_offset = (address as usize & 0x3FFF) * 2;
        (plane, byte_offset)
    }

    fn read_gdc_vram_word_from_access_page(&self, address: u32) -> u16 {
        let (plane, byte_offset) = Self::gdc_address_to_plane_and_byte_offset(address);
        if plane == 3 && !self.graphics_extension_enabled {
            return 0;
        }
        let page = self.access_page_index();
        let low = self.graphics_plane_read_byte_from_page(page, plane, byte_offset);
        let high = self.graphics_plane_read_byte_from_page(page, plane, byte_offset + 1);
        u16::from(low) | (u16::from(high) << 8)
    }
}

impl<T: TraceSink> Pc9801Bus<T> {
    fn read_byte_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u8 {
        let trace_kind = if FETCH {
            TraceAccessKind::Fetch
        } else {
            TraceAccessKind::Read
        };
        if address < 0x80000 {
            let value = self.memory.state.ram[address as usize];
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
            return value;
        }
        let address = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && let Some(value) = ga.flat_aperture_read_byte(address)
        {
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
            return value;
        }
        if address >= 0x100000 {
            let offset = (address - 0x100000) as usize;
            if offset < self.memory.extended_ram.len() {
                let value = self.memory.extended_ram[offset];
                trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
                return value;
            }
        }
        let pegc_active = self.pegc.is_256_color_active();
        let ems_b_bank = self.b_bank_ems
            && self.vram_ems_bank & 0x02 != 0
            && (0xB0000..=0xBFFFF).contains(&address);
        let in_grcg_range = !ems_b_bank
            && !pegc_active
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address));
        if self.grcg.is_active() && in_grcg_range {
            if self.is_egc_effective() {
                let value = self.egc_read_byte(address);
                trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
                return value;
            }
            let value = self.grcg_read_byte(address);
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
            return value;
        }
        if let Some(ga) = self.ga1280a.as_mut()
            && let Some(value) = ga.mapped_register_read_byte(address)
        {
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
            return value;
        }
        if !ems_b_bank
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address))
        {
            self.pending_wait_cycles += self.vram_wait;
        } else if (0xA0000..=0xA3FFF).contains(&address) {
            self.pending_wait_cycles += self.tram_wait;
        }
        let value = self.read_byte_with_access_page(address);
        trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Byte, value);
        value
    }
}

impl<T: TraceSink> common::Bus for Pc9801Bus<T> {
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
        if address < 0x80000 {
            self.memory.state.ram[address as usize] = value;
            trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
            return;
        }
        let address = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && ga.flat_aperture_write_byte(address, value)
        {
            trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
            return;
        }
        if address >= 0x100000 {
            let offset = (address - 0x100000) as usize;
            if offset < self.memory.extended_ram.len() {
                self.memory.extended_ram[offset] = value;
                trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
                return;
            }
        }
        let pegc_active = self.pegc.is_256_color_active();
        let ems_b_bank = self.b_bank_ems
            && self.vram_ems_bank & 0x02 != 0
            && (0xB0000..=0xBFFFF).contains(&address);
        let in_grcg_range = !ems_b_bank
            && !pegc_active
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address));
        if self.grcg.is_active() && in_grcg_range {
            if self.is_egc_effective() {
                self.egc_write_byte(address, value);
                trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
                return;
            }
            self.grcg_write_byte(address, value);
            trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
            return;
        }
        if let Some(ga) = self.ga1280a.as_mut()
            && ga.mapped_register_write_byte(address, value)
        {
            trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
            return;
        }
        if !ems_b_bank
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address))
        {
            self.pending_wait_cycles += self.vram_wait;
        } else if (0xA0000..=0xA3FFF).contains(&address) {
            self.pending_wait_cycles += self.tram_wait;
        }
        self.write_byte_with_access_page(address, value);
        trace_access!(T, self, MAIN_MEMORY, Write, address, Byte, value);
    }

    fn read_word(&mut self, address: u32) -> u16 {
        self.read_word_for_cpu::<false>(address)
    }

    fn write_word(&mut self, address: u32, value: u16) {
        if address.wrapping_add(1) < 0x80000 {
            let a = address as usize;
            self.memory.state.ram[a] = value as u8;
            self.memory.state.ram[a + 1] = (value >> 8) as u8;
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        let address = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && ga.flat_aperture_write_word(address, value)
        {
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        if self.machine_model.has_pegc()
            && ((0xF00000..=0xF7FFFE).contains(&address)
                || (0xFFF00000..=0xFFF7FFFE).contains(&address))
        {
            if self.pegc.is_upper_vram_enabled() {
                let vram = self.memory.state.pegc_vram.as_mut().unwrap();
                let offset = (address & 0x7FFFF) as usize;
                vram[offset] = value as u8;
                vram[offset + 1] = (value >> 8) as u8;
            }
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        if address >= 0x100000 {
            let base = (address - 0x100000) as usize;
            if base + 1 < self.memory.extended_ram.len() {
                self.memory.extended_ram[base] = value as u8;
                self.memory.extended_ram[base + 1] = (value >> 8) as u8;
                trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
                return;
            }
        }
        let pegc_active = self.pegc.is_256_color_active();
        if pegc_active && (0xA8000..=0xB7FFF).contains(&address) {
            self.pending_wait_cycles += self.vram_wait;
            if self.pegc.is_plane_mode() {
                let mut offset = address - 0xA8000;
                if self.pegc.state.screen_mode == device::pegc::PegcScreenMode::TwoScreen
                    && self.access_page_index() != 0
                {
                    offset += 0x8000;
                }
                let vram = self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice();
                self.pegc.plane_write_word(offset, value, vram);
                trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
                return;
            }
            let vram = self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice();
            let window = if address < 0xB0000 { 0 } else { 1 };
            let offset = if address < 0xB0000 {
                address - 0xA8000
            } else {
                address - 0xB0000
            };
            self.pegc.packed_write_word(window, offset, value, vram);
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        if pegc_active && (0xE0000..=0xE7FFF).contains(&address) {
            self.pending_wait_cycles += self.vram_wait;
            self.pegc.mmio_write_word(address - 0xE0000, value);
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        let ems_b_bank = self.b_bank_ems
            && self.vram_ems_bank & 0x02 != 0
            && ((0xB0000..=0xBFFFF).contains(&address)
                || (0xB0000..=0xBFFFF).contains(&(address + 1)));
        let in_grcg_range = !ems_b_bank
            && !pegc_active
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address))
            && ((0xA8000..=0xBFFFF).contains(&(address + 1))
                || (0xE0000..=0xE7FFF).contains(&(address + 1)));
        if self.grcg.is_active() && in_grcg_range {
            if self.is_egc_effective() {
                self.egc_write_word(address, value);
                trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
                return;
            }
            self.grcg_write_word(address, value);
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        if let Some(ga) = self.ga1280a.as_mut()
            && ga.mapped_register_write_word(address, value)
        {
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
            return;
        }
        if in_grcg_range {
            self.pending_wait_cycles += self.vram_wait;
        } else if (0xA0000..=0xA3FFF).contains(&address)
            && (0xA0000..=0xA3FFF).contains(&(address + 1))
        {
            self.pending_wait_cycles += self.tram_wait;
        }
        self.write_byte_with_access_page(address, value as u8);
        self.write_byte_with_access_page(address.wrapping_add(1), (value >> 8) as u8);
        trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value);
    }

    fn read_dword(&mut self, address: u32) -> u32 {
        self.read_dword_for_cpu::<false>(address)
    }

    fn write_dword(&mut self, address: u32, value: u32) {
        if address.wrapping_add(3) < 0x80000 {
            let a = address as usize;
            self.memory.state.ram[a] = value as u8;
            self.memory.state.ram[a + 1] = (value >> 8) as u8;
            self.memory.state.ram[a + 2] = (value >> 16) as u8;
            self.memory.state.ram[a + 3] = (value >> 24) as u8;
            trace_access!(T, self, MAIN_MEMORY, Write, address, Word, value as u16);
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                Write,
                address.wrapping_add(2),
                Word,
                (value >> 16) as u16
            );
            return;
        }
        let address_masked = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && ga.flat_aperture_write_dword(address_masked, value)
        {
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                Write,
                address_masked,
                Word,
                value as u16
            );
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                Write,
                address_masked.wrapping_add(2),
                Word,
                (value >> 16) as u16
            );
            return;
        }
        let pegc_active = self.pegc.is_256_color_active();
        let has_pegc = self.machine_model.has_pegc();
        let dword_end = address_masked.wrapping_add(3);
        let ems_b_bank = self.b_bank_ems
            && self.vram_ems_bank & 0x02 != 0
            && ((0xB0000..=0xBFFFF).contains(&address_masked)
                || (0xB0000..=0xBFFFF).contains(&dword_end));
        let in_grcg_range = !ems_b_bank
            && !pegc_active
            && (((0xA8000..=0xBFFFF).contains(&address_masked)
                && (0xA8000..=0xBFFFF).contains(&dword_end))
                || ((0xE0000..=0xE7FFF).contains(&address_masked)
                    && (0xE0000..=0xE7FFF).contains(&dword_end)));

        if self.grcg.is_active() && in_grcg_range && self.is_egc_effective() {
            self.egc_write_dword(address_masked, value);
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                Write,
                address_masked,
                Word,
                value as u16
            );
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                Write,
                address_masked.wrapping_add(2),
                Word,
                (value >> 16) as u16
            );
            return;
        }

        match address_masked {
            0xA8000..=0xB7FFC if pegc_active && self.pegc.is_plane_mode() => {
                self.pending_wait_cycles += self.vram_wait;
                let mut offset = address_masked - 0xA8000;
                if self.pegc.state.screen_mode == device::pegc::PegcScreenMode::TwoScreen
                    && self.access_page_index() != 0
                {
                    offset += 0x8000;
                }
                let vram = self.memory.state.pegc_vram.as_mut().unwrap().as_mut_slice();
                self.pegc.plane_write_dword(offset, value, vram);
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
            }
            0xE0000..=0xE7FFC if pegc_active => {
                self.pending_wait_cycles += self.vram_wait;
                self.pegc.mmio_write_dword(address_masked - 0xE0000, value);
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
            }
            0xF00000..=0xF7FFFC | 0xFFF00000..=0xFFF7FFFC if has_pegc => {
                if self.pegc.is_upper_vram_enabled() {
                    let vram = self.memory.state.pegc_vram.as_mut().unwrap();
                    let offset = (address_masked & 0x7FFFF) as usize;
                    vram[offset] = value as u8;
                    vram[offset + 1] = (value >> 8) as u8;
                    vram[offset + 2] = (value >> 16) as u8;
                    vram[offset + 3] = (value >> 24) as u8;
                }
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    Write,
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
            }
            _ => {
                if address_masked >= 0x100000 {
                    let base = (address_masked - 0x100000) as usize;
                    if base + 3 < self.memory.extended_ram.len() {
                        self.memory.extended_ram[base] = value as u8;
                        self.memory.extended_ram[base + 1] = (value >> 8) as u8;
                        self.memory.extended_ram[base + 2] = (value >> 16) as u8;
                        self.memory.extended_ram[base + 3] = (value >> 24) as u8;
                        trace_access!(
                            T,
                            self,
                            MAIN_MEMORY,
                            Write,
                            address_masked,
                            Word,
                            value as u16
                        );
                        trace_access!(
                            T,
                            self,
                            MAIN_MEMORY,
                            Write,
                            address_masked.wrapping_add(2),
                            Word,
                            (value >> 16) as u16
                        );
                        return;
                    }
                }
                self.write_word(address, value as u16);
                self.write_word(address.wrapping_add(2), (value >> 16) as u16);
            }
        }
    }

    fn io_read_byte(&mut self, port: u16) -> u8 {
        self.io_read_byte_impl(port)
    }

    fn io_write_byte(&mut self, port: u16, value: u8) {
        self.io_write_byte_impl(port, value)
    }

    fn io_read_word(&mut self, port: u16) -> u16 {
        // I-O DATA GA-1280A: word-only when the GA has an atomic
        // word handler. When it does not, fall through to the byte-split path
        // so the byte handlers compose the result.
        if is_ga1280a_port(port)
            && let Some(value) = self
                .ga1280a
                .as_mut()
                .and_then(|ga| ga.try_handle_io_read_word(port))
        {
            self.pending_wait_cycles += IO_WAIT_CYCLES + self.cbus_wait_cycles();
            trace_access!(T, self, MAIN_IO, Read, port, Word, value);
            return value;
        }

        match port {
            // IDE 16-bit data register.
            0x0640 if self.machine_model.has_ide() => {
                self.pending_wait_cycles += IO_WAIT_CYCLES;
                let (word, action) = self.ide.read_data_word();
                self.process_ide_action(action);
                trace_access!(T, self, MAIN_IO, Read, port, Word, word);
                word
            }
            _ => {
                let low = self.io_read_byte(port) as u16;
                let high = self.io_read_byte(port.wrapping_add(1)) as u16;
                low | (high << 8)
            }
        }
    }

    fn io_write_word(&mut self, port: u16, value: u16) {
        // I-O DATA GA-1280A: word-only when the GA has an atomic
        // word handler. When it does not, fall through to the byte-split path.
        if is_ga1280a_port(port)
            && self
                .ga1280a
                .as_mut()
                .is_some_and(|ga| ga.try_handle_io_write_word(port, value))
        {
            self.pending_wait_cycles += IO_WAIT_CYCLES + self.cbus_wait_cycles();
            trace_access!(T, self, MAIN_IO, Write, port, Word, value);
            return;
        }

        match port {
            // IDE 16-bit data register.
            0x0640 if self.machine_model.has_ide() => {
                self.pending_wait_cycles += IO_WAIT_CYCLES;
                let action = self.ide.write_data_word(value);
                self.process_ide_action(action);
                trace_access!(T, self, MAIN_IO, Write, port, Word, value);
            }
            // EGC registers: atomic word write avoids double recalculate_shift()
            // that the default byte-split path would cause on shift (0x04AC) and
            // length (0x04AE) registers.
            0x04A0..=0x04AE if port & 1 == 0 => {
                self.pending_wait_cycles += IO_WAIT_CYCLES;

                if self.machine_model.has_egc()
                    && self.display_control.is_egc_extended_mode_effective()
                    && self.grcg.is_active()
                {
                    self.egc.write_register_word((port & 0x0F) as u8, value);
                }
            }
            _ => {
                self.io_write_byte(port, value as u8);
                self.io_write_byte(port.wrapping_add(1), (value >> 8) as u8);
            }
        }
    }

    fn is_io_port_unrestricted(&self, port: u16) -> bool {
        matches!(port, 0x07EE..=0x07F0)
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
                    trace_id::controller::PC98_PIC,
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

    fn drain_wait_cycles(&mut self) -> i64 {
        let cycles = self.pending_wait_cycles;
        self.pending_wait_cycles = 0;
        cycles
    }

    fn reset_pending(&self) -> bool {
        self.reset_pending
    }

    fn signal_fpu_error(&mut self) {
        // PC-98: FERR# is routed to IRQ 8 (slave PIC IR0).
        self.raise_pic_irq(8);
    }

    fn cpu_should_yield(&self) -> bool {
        T::ENABLED && self.tracer.yield_requested()
            || self.sasi.take_yield_requested()
            || self.ide.take_yield_requested()
            || self.fdd640k_hle.take_yield_requested()
            || self.bios.take_yield_requested()
    }
}

impl<T: TraceSink> Pc9801Bus<T> {
    /// Reads one word while preserving PC-98 wide-access behavior and trace kind.
    fn read_word_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u16 {
        let trace_kind = if FETCH {
            TraceAccessKind::Fetch
        } else {
            TraceAccessKind::Read
        };
        if address.wrapping_add(1) < 0x80000 {
            let a = address as usize;
            let value =
                self.memory.state.ram[a] as u16 | ((self.memory.state.ram[a + 1] as u16) << 8);
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        let address = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && let Some(value) = ga.flat_aperture_read_word(address)
        {
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        if self.machine_model.has_pegc()
            && ((0xF00000..=0xF7FFFE).contains(&address)
                || (0xFFF00000..=0xFFF7FFFE).contains(&address))
        {
            let value = if self.pegc.is_upper_vram_enabled() {
                let vram = self.memory.state.pegc_vram.as_ref().unwrap();
                let offset = (address & 0x7FFFF) as usize;
                vram[offset] as u16 | ((vram[offset + 1] as u16) << 8)
            } else {
                0xFFFF
            };
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        if address >= 0x100000 {
            let base = (address - 0x100000) as usize;
            if base + 1 < self.memory.extended_ram.len() {
                let value = self.memory.extended_ram[base] as u16
                    | ((self.memory.extended_ram[base + 1] as u16) << 8);
                trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
                return value;
            }
        }
        let pegc_active = self.pegc.is_256_color_active();
        if pegc_active && (0xA8000..=0xB7FFF).contains(&address) {
            self.pending_wait_cycles += self.vram_wait;
            if self.pegc.is_plane_mode() {
                let mut offset = address - 0xA8000;
                if self.pegc.state.screen_mode == device::pegc::PegcScreenMode::TwoScreen
                    && self.access_page_index() != 0
                {
                    offset += 0x8000;
                }
                let vram = self.memory.state.pegc_vram.as_ref().unwrap().as_slice();
                let value = self.pegc.plane_read_word(offset, vram);
                trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
                return value;
            }
            let vram = self.memory.state.pegc_vram.as_ref().unwrap().as_slice();
            let window = if address < 0xB0000 { 0 } else { 1 };
            let offset = if address < 0xB0000 {
                address - 0xA8000
            } else {
                address - 0xB0000
            };
            let value = self.pegc.packed_read_word(window, offset, vram);
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        if pegc_active && (0xE0000..=0xE7FFF).contains(&address) {
            self.pending_wait_cycles += self.vram_wait;
            let value = self.pegc.mmio_read_word(address - 0xE0000);
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        let ems_b_bank = self.b_bank_ems
            && self.vram_ems_bank & 0x02 != 0
            && ((0xB0000..=0xBFFFF).contains(&address)
                || (0xB0000..=0xBFFFF).contains(&(address + 1)));
        let in_grcg_range = !ems_b_bank
            && !pegc_active
            && ((0xA8000..=0xBFFFF).contains(&address) || (0xE0000..=0xE7FFF).contains(&address))
            && ((0xA8000..=0xBFFFF).contains(&(address + 1))
                || (0xE0000..=0xE7FFF).contains(&(address + 1)));
        if self.grcg.is_active() && in_grcg_range {
            if self.is_egc_effective() {
                let value = self.egc_read_word(address);
                trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
                return value;
            }
            let value = self.grcg_read_word(address);
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        if let Some(ga) = self.ga1280a.as_mut()
            && let Some(value) = ga.mapped_register_read_word(address)
        {
            trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
            return value;
        }
        if in_grcg_range {
            self.pending_wait_cycles += self.vram_wait;
        } else if (0xA0000..=0xA3FFF).contains(&address)
            && (0xA0000..=0xA3FFF).contains(&(address + 1))
        {
            self.pending_wait_cycles += self.tram_wait;
        }
        let low = self.read_byte_with_access_page(address) as u16;
        let high = self.read_byte_with_access_page(address.wrapping_add(1)) as u16;
        let value = low | (high << 8);
        trace_access!(T, self, MAIN_MEMORY, (trace_kind), address, Word, value);
        value
    }

    /// Reads one doubleword as PC-98 bus transactions with the requested trace kind.
    fn read_dword_for_cpu<const FETCH: bool>(&mut self, address: u32) -> u32 {
        let trace_kind = if FETCH {
            TraceAccessKind::Fetch
        } else {
            TraceAccessKind::Read
        };
        if address.wrapping_add(3) < 0x80000 {
            let a = address as usize;
            let value = self.memory.state.ram[a] as u32
                | ((self.memory.state.ram[a + 1] as u32) << 8)
                | ((self.memory.state.ram[a + 2] as u32) << 16)
                | ((self.memory.state.ram[a + 3] as u32) << 24);
            if T::ENABLED {
                self.trace_dword_words(trace_kind, address, value);
            }
            return value;
        }
        let address_masked = self.a20_mask(address);
        if let Some(ga) = self.ga1280a.as_mut()
            && let Some(value) = ga.flat_aperture_read_dword(address_masked)
        {
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                (trace_kind),
                address_masked,
                Word,
                value as u16
            );
            trace_access!(
                T,
                self,
                MAIN_MEMORY,
                (trace_kind),
                address_masked.wrapping_add(2),
                Word,
                (value >> 16) as u16
            );
            return value;
        }
        let pegc_active = self.pegc.is_256_color_active();
        let has_pegc = self.machine_model.has_pegc();

        match address_masked {
            0xA8000..=0xB7FFC if pegc_active && self.pegc.is_plane_mode() => {
                self.pending_wait_cycles += self.vram_wait;
                let mut offset = address_masked - 0xA8000;
                if self.pegc.state.screen_mode == device::pegc::PegcScreenMode::TwoScreen
                    && self.access_page_index() != 0
                {
                    offset += 0x8000;
                }
                let vram = self.memory.state.pegc_vram.as_ref().unwrap().as_slice();
                let value = self.pegc.plane_read_dword(offset, vram);
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
                value
            }
            0xE0000..=0xE7FFC if pegc_active => {
                self.pending_wait_cycles += self.vram_wait;
                let value = self.pegc.mmio_read_dword(address_masked - 0xE0000);
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
                value
            }
            0xF00000..=0xF7FFFC | 0xFFF00000..=0xFFF7FFFC if has_pegc => {
                let value = if self.pegc.is_upper_vram_enabled() {
                    let vram = self.memory.state.pegc_vram.as_ref().unwrap();
                    let offset = (address_masked & 0x7FFFF) as usize;
                    vram[offset] as u32
                        | ((vram[offset + 1] as u32) << 8)
                        | ((vram[offset + 2] as u32) << 16)
                        | ((vram[offset + 3] as u32) << 24)
                } else {
                    0xFFFF_FFFF
                };
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked,
                    Word,
                    value as u16
                );
                trace_access!(
                    T,
                    self,
                    MAIN_MEMORY,
                    (trace_kind),
                    address_masked.wrapping_add(2),
                    Word,
                    (value >> 16) as u16
                );
                value
            }
            _ => {
                if address_masked >= 0x100000 {
                    let base = (address_masked - 0x100000) as usize;
                    if base + 3 < self.memory.extended_ram.len() {
                        let value = self.memory.extended_ram[base] as u32
                            | ((self.memory.extended_ram[base + 1] as u32) << 8)
                            | ((self.memory.extended_ram[base + 2] as u32) << 16)
                            | ((self.memory.extended_ram[base + 3] as u32) << 24);
                        if T::ENABLED {
                            self.trace_dword_words(trace_kind, address_masked, value);
                        }
                        return value;
                    }
                }
                let low = self.read_word_for_cpu::<FETCH>(address) as u32;
                let high = self.read_word_for_cpu::<FETCH>(address.wrapping_add(2)) as u32;
                low | (high << 16)
            }
        }
    }

    /// Emits the two 16-bit transactions used for a PC-98 doubleword access.
    fn trace_dword_words(&mut self, kind: TraceAccessKind, address: u32, value: u32) {
        trace_access!(T, self, MAIN_MEMORY, (kind), address, Word, value as u16);
        trace_access!(
            T,
            self,
            MAIN_MEMORY,
            (kind),
            address.wrapping_add(2),
            Word,
            (value >> 16) as u16
        );
    }
}

#[cfg(test)]
mod tests {
    use common::{
        Bus, CpuMode, MachineModel, TraceAccess, TraceAccessKind, TraceAccessWidth, TraceContext,
        TraceEvent, TraceInterrupt, TraceInterruptAction, TraceSink,
    };
    use device::disk::{HddFormat, HddGeometry, HddImage};

    use super::{NoTrace, Pc9801Bus};
    use crate::scheduler::Event98;

    const GAINIT_WINDOW_BASE: u32 = 0xC0000;
    const GAINIT_WINDOW_BLOCK_SIZE: u32 = 0x1000;
    const GAINIT_WINDOW_BLOCK_COUNT: usize = 32;

    #[derive(Default)]
    struct AccessTrace {
        kinds: Vec<TraceAccessKind>,
        accesses: Vec<TraceAccess>,
        scheduled_contexts: Vec<TraceContext>,
        interrupts: Vec<TraceInterrupt>,
    }

    impl TraceSink for AccessTrace {
        fn trace(&mut self, context: TraceContext, event: TraceEvent<'_>) {
            match event {
                TraceEvent::Access(access) => {
                    self.kinds.push(access.kind);
                    self.accesses.push(access);
                }
                TraceEvent::Scheduled { .. } => self.scheduled_contexts.push(context),
                TraceEvent::Interrupt(interrupt) => self.interrupts.push(interrupt),
                _ => {}
            }
        }
    }

    #[test]
    fn opcode_fetch_is_distinct_from_data_read() {
        let mut bus = Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801VM,
            CpuMode::Low,
            48_000,
            AccessTrace::default(),
        );

        Bus::read_byte(&mut bus, 0);
        Bus::fetch_opcode_byte(&mut bus, 1);

        assert_eq!(
            bus.tracer().kinds,
            [TraceAccessKind::Read, TraceAccessKind::Fetch]
        );
    }

    #[test]
    fn wide_opcode_fetches_keep_pc98_bus_transaction_width() {
        let mut bus = Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801RA,
            CpuMode::High,
            48_000,
            AccessTrace::default(),
        );

        Bus::fetch_opcode_word(&mut bus, 0);
        Bus::fetch_opcode_dword(&mut bus, 4);

        assert_eq!(bus.tracer().accesses.len(), 3);
        assert_eq!(bus.tracer().accesses[0].kind, TraceAccessKind::Fetch);
        assert_eq!(bus.tracer().accesses[0].width, TraceAccessWidth::Word);
        assert!(
            bus.tracer().accesses[1..]
                .iter()
                .all(|access| access.kind == TraceAccessKind::Fetch)
        );
        assert!(
            bus.tracer().accesses[1..]
                .iter()
                .all(|access| access.width == TraceAccessWidth::Word)
        );
        assert_eq!(bus.tracer().accesses[1].address, 4);
        assert_eq!(bus.tracer().accesses[2].address, 6);
    }

    /// Confirms optimized doubleword traces use the address after A20 masking.
    #[test]
    fn a20_masked_dword_accesses_trace_bus_addresses() {
        let aliased_address = 0x30_0000;
        let bus_address = 0x20_0000;
        let value = 0x1234_ABCD;

        let mut bus = Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801RA,
            CpuMode::High,
            48_000,
            AccessTrace::default(),
        );

        Bus::write_dword(&mut bus, aliased_address, value);
        assert_eq!(Bus::read_dword(&mut bus, bus_address), value);
        assert_eq!(Bus::fetch_opcode_dword(&mut bus, aliased_address), value);

        assert_eq!(bus.tracer().accesses.len(), 6);
        for accesses in bus.tracer().accesses.chunks_exact(2) {
            assert_eq!(accesses[0].address, u64::from(bus_address));
            assert_eq!(accesses[1].address, u64::from(bus_address + 2));
            assert!(
                accesses
                    .iter()
                    .all(|access| access.width == TraceAccessWidth::Word)
            );
        }
        assert_eq!(
            bus.tracer().kinds,
            [
                TraceAccessKind::Write,
                TraceAccessKind::Write,
                TraceAccessKind::Read,
                TraceAccessKind::Read,
                TraceAccessKind::Fetch,
                TraceAccessKind::Fetch,
            ]
        );
    }

    #[test]
    fn scheduler_context_has_explicit_source_and_clock() {
        let mut bus = Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801VM,
            CpuMode::Low,
            48_000,
            AccessTrace::default(),
        );
        bus.scheduler.schedule(Event98::GdcDrawingComplete, 17);
        bus.next_event_cycle = 17;

        Bus::set_current_cycle(&mut bus, 17);

        assert_eq!(bus.tracer().scheduled_contexts.len(), 1);
        assert_eq!(
            bus.tracer().scheduled_contexts[0],
            TraceContext::scheduler_main(17, Some(u64::from(bus.cpu_clock_hz())))
        );
    }

    #[test]
    fn keyboard_and_serial_irq_traces_only_include_transitions() {
        let mut bus = Pc9801Bus::new_with_trace_sink(
            MachineModel::PC9801VM,
            CpuMode::Low,
            48_000,
            AccessTrace::default(),
        );

        bus.push_keyboard_scancode(0x1C);
        bus.push_keyboard_scancode(0x9C);
        bus.push_serial_byte(0x41);
        bus.push_serial_byte(0x42);
        bus.clear_pic_irq(1);
        bus.clear_pic_irq(1);
        bus.clear_pic_irq(4);
        bus.clear_pic_irq(4);

        let transitions: Vec<_> = bus
            .tracer()
            .interrupts
            .iter()
            .map(|interrupt| (interrupt.line, interrupt.action))
            .collect();
        assert_eq!(
            transitions,
            [
                (Some(1), TraceInterruptAction::Assert),
                (Some(4), TraceInterruptAction::Assert),
                (Some(1), TraceInterruptAction::Clear),
                (Some(4), TraceInterruptAction::Clear),
            ]
        );
    }

    fn gainit_window_block_occupied(
        block_index: usize,
        read_word: &mut impl FnMut(u32) -> u16,
    ) -> bool {
        let base = GAINIT_WINDOW_BASE + block_index as u32 * GAINIT_WINDOW_BLOCK_SIZE;
        for offset in (0..GAINIT_WINDOW_BLOCK_SIZE).step_by(2) {
            let address = base + offset;
            if read_word(address) != 0xFFFF {
                let mut stable = true;
                for _ in 0..31 {
                    if read_word(address) == 0xFFFF {
                        stable = false;
                        break;
                    }
                }
                if stable {
                    return true;
                }
            }
        }
        false
    }

    fn gainit_choose_window_segment(
        required_blocks: usize,
        mut read_word: impl FnMut(u32) -> u16,
    ) -> Option<u16> {
        assert!((1..=GAINIT_WINDOW_BLOCK_COUNT).contains(&required_blocks));
        let last_candidate = GAINIT_WINDOW_BLOCK_COUNT - required_blocks;
        let mut candidate_block = 0;

        while candidate_block <= last_candidate {
            let mut occupied = false;
            for block in candidate_block..candidate_block + required_blocks {
                if gainit_window_block_occupied(block, &mut read_word) {
                    occupied = true;
                    break;
                }
            }
            if !occupied {
                return Some(0xC000 + candidate_block as u16 * 0x0100);
            }
            candidate_block += 4;
        }

        None
    }

    fn gainit_choose_window_from_occupancy(
        occupied_blocks: &[bool; GAINIT_WINDOW_BLOCK_COUNT],
        required_blocks: usize,
    ) -> Option<u16> {
        gainit_choose_window_segment(required_blocks, |address| {
            let block = ((address - GAINIT_WINDOW_BASE) / GAINIT_WINDOW_BLOCK_SIZE) as usize;
            if occupied_blocks[block] {
                0x0000
            } else {
                0xFFFF
            }
        })
    }

    fn compose_halfwidth_font_address(
        video_mode: u8,
        attr_byte: u8,
        char_low: u8,
        glyph_y_16: u32,
    ) -> u32 {
        let font_select_8x16 = (video_mode & 0x08) != 0;
        let attr_semigraphics_mode = (video_mode & 0x01) != 0;
        let semigraphics = attr_semigraphics_mode && ((attr_byte & 0x10) != 0);

        if font_select_8x16 {
            let mut font_base = 0x80000 + u32::from(char_low) * 16 + glyph_y_16;
            if semigraphics {
                font_base += 0x1000;
            }
            font_base
        } else {
            let font_line = glyph_y_16 / 2;
            let mut font_base = 0x82000 + u32::from(char_low) * 16 + font_line;
            if semigraphics {
                font_base += 8;
            }
            font_base
        }
    }

    #[test]
    fn gainit_window_probe_selects_c000_on_clean_bus() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        assert_eq!(
            gainit_choose_window_segment(4, |address| bus.read_word(address)),
            Some(0xC000),
            "A clean expansion-ROM gap should let default GAINIT choose C000h"
        );
    }

    #[test]
    fn gainit_window_probe_skips_occupied_16k_candidate() {
        let mut occupied_blocks = [false; GAINIT_WINDOW_BLOCK_COUNT];
        occupied_blocks[..4].fill(true);

        assert_eq!(
            gainit_choose_window_from_occupancy(&occupied_blocks, 4),
            Some(0xC400),
            "GAINIT must skip the first 16 KB slot when any block in it is busy"
        );
    }

    #[test]
    fn gainit_window_probe_requires_contiguous_blocks_for_large_windows() {
        let mut occupied_blocks = [false; GAINIT_WINDOW_BLOCK_COUNT];
        occupied_blocks[8] = true;

        assert_eq!(
            gainit_choose_window_from_occupancy(&occupied_blocks, 16),
            Some(0xCC00),
            "Larger windows need one uninterrupted run of free 4 KB blocks"
        );
    }

    #[test]
    fn gainit_window_probe_returns_none_when_all_blocks_are_occupied() {
        let occupied_blocks = [true; GAINIT_WINDOW_BLOCK_COUNT];

        assert_eq!(
            gainit_choose_window_from_occupancy(&occupied_blocks, 4),
            None,
            "If no candidate range is free, GAINIT should fail instead of guessing"
        );
    }

    #[test]
    fn gainit_window_probe_treats_ems_page_frame_as_occupied() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
        bus.memory.enable_ems_page_frame();

        assert_eq!(
            gainit_choose_window_segment(4, |address| bus.read_word(address)),
            Some(0xD000),
            "EMS page-frame RAM must block C000h-CFFFFh from being reused by GA."
        );
    }

    #[test]
    fn gainit_window_probe_treats_umb_region_as_occupied_for_128k_window() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);

        assert_eq!(
            gainit_choose_window_segment(32, |address| bus.read_word(address)),
            Some(0xC000)
        );

        bus.memory.enable_umb_region(None);

        assert_eq!(
            gainit_choose_window_segment(32, |address| bus.read_word(address)),
            None,
            "A 128 KB GA window cannot fit once UMB occupies C0000h-DFFFFh"
        );
    }

    #[test]
    fn umb_region_overrides_hle_expansion_rom_overlay_reads() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AP, CpuMode::Low, 48000);
        let geometry = HddGeometry {
            cylinders: 1,
            heads: 1,
            sectors_per_track: 1,
            sector_size: 512,
        };
        let image = HddImage::from_raw(geometry, HddFormat::Hdi, vec![0; 512]);
        bus.insert_hdd(0, image, None);

        bus.write_byte(0xD8000, 0x5A);
        assert_ne!(
            bus.read_byte(0xD8000),
            0x5A,
            "Before UMBs are enabled, the IDE HLE ROM overlay owns D8000h."
        );

        bus.memory.enable_umb_region(None);
        bus.write_byte(0xD8000, 0xA5);

        assert_eq!(
            bus.read_byte(0xD8000),
            0xA5,
            "After UMBs are enabled, D8000h must read the RAM page DOS writes."
        );
    }

    #[test]
    fn render_display_frame_uses_active_display_page_for_graphics() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        // Enable global display (mode1 bit 7), 16-color/analog palette (mode2 bit 0),
        // and graphics display.
        bus.display_control.state.video_mode = 0x80;
        bus.display_control.state.mode2 |= 0x01;
        bus.gdc_slave.state.display_enabled = true;
        bus.gdc_slave.state.pitch = 40;
        bus.gdc_slave.state.scroll[0].line_count = 400;
        bus.gdc_slave.state.lines_per_row = 1;
        // palette[2] = red. With only the R plane bit set, graphics_color resolves
        // to (R<<1) = 2.
        bus.palette.state.analog[2] = [0x00, 0x0F, 0x00];

        // Page 0: top-left pixel reads bit 7 of R-plane, set to 1.
        bus.memory.state.graphics_vram[0x8000] = 0x80;

        bus.render_display_frame();
        let fb = bus.display_framebuffer();
        assert_eq!(&fb[0..4], &[0xFF, 0x00, 0x00, 0xFF], "page 0 red pixel");

        // Switch display page to 1 - top-left should now read page 1 R-plane, which is 0.
        bus.io_write_byte(0xA4, 0x01);
        bus.render_display_frame();
        let fb = bus.display_framebuffer();
        assert_eq!(&fb[0..4], &[0x00, 0x00, 0x00, 0xFF], "page 1 black pixel");
    }

    #[test]
    fn compose_halfwidth_font_address_follows_8x16_6x8_and_attr_bit0_contract() {
        let char_code = 0x34;
        let glyph_y = 13;
        let attr_with_bit4 = 0x10;
        let attr_without_bit4 = 0x00;

        // 8x16 mode, attr semigraphics disabled: bit4 is vertical-line attribute, not semigraphics.
        assert_eq!(
            compose_halfwidth_font_address(0x08, attr_with_bit4, char_code, glyph_y),
            0x80000 + 0x34 * 16 + 13
        );
        assert_eq!(
            compose_halfwidth_font_address(0x08, attr_without_bit4, char_code, glyph_y),
            0x80000 + 0x34 * 16 + 13
        );

        // 8x16 mode, attr semigraphics enabled: bit4 selects chargraph16 bank.
        assert_eq!(
            compose_halfwidth_font_address(0x09, attr_with_bit4, char_code, glyph_y),
            0x81000 + 0x34 * 16 + 13
        );
        assert_eq!(
            compose_halfwidth_font_address(0x09, attr_without_bit4, char_code, glyph_y),
            0x80000 + 0x34 * 16 + 13
        );

        // 6x8 mode halves glyph line index.
        assert_eq!(
            compose_halfwidth_font_address(0x00, attr_with_bit4, char_code, glyph_y),
            0x82000 + 0x34 * 16 + 6
        );
        assert_eq!(
            compose_halfwidth_font_address(0x01, attr_with_bit4, char_code, glyph_y),
            0x82000 + 0x34 * 16 + 8 + 6
        );
    }

    #[test]
    fn e_plane_read_charges_vram_wait() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
        bus.set_graphics_extension_enabled(true);

        bus.pending_wait_cycles = 0;
        let _ = bus.read_byte(0xE0000);
        assert!(
            bus.pending_wait_cycles > 0,
            "E-plane read should charge vram_wait"
        );
    }

    #[test]
    fn e_plane_write_charges_vram_wait() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
        bus.set_graphics_extension_enabled(true);

        bus.pending_wait_cycles = 0;
        bus.write_byte(0xE0000, 0x42);
        assert!(
            bus.pending_wait_cycles > 0,
            "E-plane write should charge vram_wait"
        );
    }

    #[test]
    fn access_page_selects_vram_bank_for_cpu_writes() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        // Write to page 0 (default).
        bus.write_byte(0xA8000, 0xAA);
        assert_eq!(bus.memory.state.graphics_vram[0], 0xAA);

        // Switch to page 1 via port 0xA6.
        bus.io_write_byte(0xA6, 0x01);
        bus.write_byte(0xA8000, 0xBB);

        // Page 0 unchanged, page 1 written.
        assert_eq!(bus.memory.state.graphics_vram[0], 0xAA);
        let page1_base = super::GRAPHICS_PAGE_SIZE_BYTES;
        assert_eq!(bus.memory.state.graphics_vram[page1_base], 0xBB);
    }

    #[test]
    fn access_page_selects_vram_bank_for_cpu_reads() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);

        let page1_base = super::GRAPHICS_PAGE_SIZE_BYTES;
        bus.memory.state.graphics_vram[0] = 0x11;
        bus.memory.state.graphics_vram[page1_base] = 0x22;

        assert_eq!(bus.read_byte(0xA8000), 0x11);

        bus.io_write_byte(0xA6, 0x01);
        assert_eq!(bus.read_byte(0xA8000), 0x22);

        bus.io_write_byte(0xA6, 0x00);
        assert_eq!(bus.read_byte(0xA8000), 0x11);
    }

    #[test]
    fn access_page_selects_e_plane_bank() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801VM, CpuMode::Low, 48000);
        bus.display_control.state.mode2 |= 0x01;
        bus.set_graphics_extension_enabled(true);

        let e_page1_base = super::E_PLANE_PAGE_SIZE_BYTES;
        bus.memory.state.e_plane_vram[0] = 0x33;
        bus.memory.state.e_plane_vram[e_page1_base] = 0x44;

        assert_eq!(bus.read_byte(0xE0000), 0x33);

        bus.io_write_byte(0xA6, 0x01);
        assert_eq!(bus.read_byte(0xE0000), 0x44);

        bus.write_byte(0xE0000, 0x55);
        assert_eq!(bus.memory.state.e_plane_vram[e_page1_base], 0x55);
        assert_eq!(bus.memory.state.e_plane_vram[0], 0x33);
    }

    #[test]
    fn ram_window_default_identity() {
        let bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        assert_eq!(bus.ram_window, 0x08);
    }

    #[test]
    fn ram_window_remaps_to_extended_ram() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        // Set RAM window to 0x10 -> physical base 0x100000 (1 MB).
        bus.ram_window = 0x10;
        // Write via remapped window.
        bus.write_byte_with_access_page(0x80000, 0xAB);
        // Should land in extended RAM at offset 0.
        assert_eq!(bus.memory.state.extended_ram[0], 0xAB);
        // Read back via remapped window.
        assert_eq!(bus.read_byte_with_access_page(0x80000), 0xAB);
        // Original RAM at 0x80000 should be untouched.
        assert_eq!(bus.memory.state.ram[0x80000], 0x00);
    }

    fn create_pc9821_bus() -> Pc9801Bus<NoTrace> {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
        bus.display_control.state.mode2 |= 0x01 | 0x08;
        bus.set_graphics_extension_enabled(true);
        bus
    }

    fn enable_pegc(bus: &mut Pc9801Bus<NoTrace>) {
        bus.io_write_byte(0x6A, 0x21);
    }

    fn disable_pegc(bus: &mut Pc9801Bus<NoTrace>) {
        bus.io_write_byte(0x6A, 0x20);
    }

    #[test]
    fn pegc_port_6a_0x21_enables_256_color() {
        let mut bus = create_pc9821_bus();
        assert!(!bus.pegc.is_256_color_active());
        enable_pegc(&mut bus);
        assert!(bus.pegc.is_256_color_active());
    }

    #[test]
    fn pegc_port_6a_0x20_disables_256_color() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        assert!(bus.pegc.is_256_color_active());
        disable_pegc(&mut bus);
        assert!(!bus.pegc.is_256_color_active());
    }

    #[test]
    fn pegc_port_6a_screen_mode() {
        let mut bus = create_pc9821_bus();
        bus.io_write_byte(0x6A, 0x69);
        assert_eq!(
            bus.pegc.state.screen_mode,
            device::pegc::PegcScreenMode::OneScreen
        );
        bus.io_write_byte(0x6A, 0x68);
        assert_eq!(
            bus.pegc.state.screen_mode,
            device::pegc::PegcScreenMode::TwoScreen
        );
    }

    #[test]
    fn pegc_port_6a_ignored_on_non_9821() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9801RA, CpuMode::Low, 48000);
        bus.io_write_byte(0x6A, 0x21);
        assert!(!bus.pegc.is_256_color_active());
    }

    #[test]
    fn pegc_e0000_routes_to_mmio_when_active() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.write_byte(0xE0004, 0x05);
        assert_eq!(bus.pegc.state.bank_a8, 0x05);
        assert_eq!(bus.read_byte(0xE0004), 0x05);
    }

    #[test]
    fn pegc_e0000_routes_to_e_plane_when_inactive() {
        let mut bus = create_pc9821_bus();

        bus.memory.state.e_plane_vram[0] = 0xAB;
        assert_eq!(bus.read_byte(0xE0000), 0xAB);

        enable_pegc(&mut bus);
        assert_ne!(bus.read_byte(0xE0000), 0xAB);

        disable_pegc(&mut bus);
        assert_eq!(bus.read_byte(0xE0000), 0xAB);
    }

    #[test]
    fn pegc_a8000_routes_to_pegc_vram_when_active() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.write_byte(0xA8000, 0x42);
        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0], 0x42);
        assert_eq!(bus.read_byte(0xA8000), 0x42);
    }

    #[test]
    fn pegc_b0000_routes_to_pegc_vram_when_active() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.pegc.state.bank_b0 = 1;
        bus.write_byte(0xB0000, 0x77);
        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0x8000], 0x77);
        assert_eq!(bus.read_byte(0xB0000), 0x77);
    }

    #[test]
    fn pegc_grcg_bypassed_when_active() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.grcg.write_mode(0x80);
        bus.grcg.write_tile(0xFF);
        bus.grcg.write_tile(0xFF);
        bus.grcg.write_tile(0xFF);
        bus.grcg.write_tile(0xFF);

        bus.write_byte(0xA8000, 0x42);

        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0], 0x42);
        assert_eq!(bus.memory.state.graphics_vram[0], 0x00);
    }

    #[test]
    fn pegc_flat_access_f00000() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        bus.a20_enabled = true;

        bus.write_word(0xE0102, 0x0001);
        assert!(bus.pegc.is_upper_vram_enabled());

        bus.write_byte(0xF00000, 0xAA);
        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0], 0xAA);
        assert_eq!(bus.read_byte(0xF00000), 0xAA);

        bus.write_byte(0xF7FFFF, 0xBB);
        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0x7FFFF], 0xBB);
    }

    #[test]
    fn pegc_flat_access_disabled_by_default() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        bus.a20_enabled = true;

        bus.memory.state.pegc_vram.as_mut().unwrap()[0] = 0xCC;

        let value = bus.read_byte(0xF00000);
        assert_eq!(value, 0xFF);
    }

    #[test]
    fn pegc_palette_ports_route_to_pegc_when_active() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.io_write_byte(0xA8, 100);
        bus.io_write_byte(0xAA, 0x11);
        bus.io_write_byte(0xAC, 0x22);
        bus.io_write_byte(0xAE, 0x33);

        assert_eq!(bus.pegc.state.palette_index, 100);
        assert_eq!(bus.pegc.state.palette_256[100], [0x11, 0x22, 0x33]);
    }

    #[test]
    fn pegc_palette_ports_route_to_analog_when_inactive() {
        let mut bus = create_pc9821_bus();

        bus.io_write_byte(0xA8, 5);
        bus.io_write_byte(0xAA, 0x0A);

        assert_eq!(bus.palette.state.index, 5);
        assert_eq!(bus.palette.state.analog[5][0], 0x0A);
    }

    #[test]
    fn pegc_b8000_falls_through_to_graphics_vram() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        let page_base = bus.access_page_index() * super::GRAPHICS_PAGE_SIZE_BYTES;
        bus.memory.state.graphics_vram[page_base + (0xB8000 - 0xA8000)] = 0xCD;

        assert_eq!(bus.read_byte(0xB8000), 0xCD);
    }

    #[test]
    fn pegc_b7fff_routes_to_pegc_vram() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.write_byte(0xB7FFF, 0xAB);
        assert_eq!(bus.read_byte(0xB7FFF), 0xAB);

        let vram = bus.memory.state.pegc_vram.as_ref().unwrap();
        assert_eq!(vram[0x7FFF], 0xAB);
    }

    #[test]
    fn pegc_palette_read_256_color_mode() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.io_write_byte(0xA8, 42);
        bus.io_write_byte(0xAA, 0x11);
        bus.io_write_byte(0xAC, 0x22);
        bus.io_write_byte(0xAE, 0x33);

        bus.io_write_byte(0xA8, 42);
        assert_eq!(bus.io_read_byte(0xA8), 42);
        assert_eq!(bus.io_read_byte(0xAA), 0x11);
        assert_eq!(bus.io_read_byte(0xAC), 0x22);
        assert_eq!(bus.io_read_byte(0xAE), 0x33);
    }

    #[test]
    fn pegc_palette_read_analog_mode() {
        let mut bus = create_pc9821_bus();

        bus.io_write_byte(0xA8, 5);
        bus.io_write_byte(0xAA, 0x0A);
        bus.io_write_byte(0xAC, 0x0B);
        bus.io_write_byte(0xAE, 0x0C);

        assert_eq!(bus.io_read_byte(0xA8), 5);
        assert_eq!(bus.io_read_byte(0xAA), 0x0A);
        assert_eq!(bus.io_read_byte(0xAC), 0x0B);
        assert_eq!(bus.io_read_byte(0xAE), 0x0C);
    }

    #[test]
    fn pegc_port_6a_0x21_blocked_without_mode2_bit3() {
        let mut bus = Pc9801Bus::<NoTrace>::new(MachineModel::PC9821AS, CpuMode::Low, 48000);
        bus.display_control.state.mode2 |= 0x01;
        bus.set_graphics_extension_enabled(true);
        bus.io_write_byte(0x6A, 0x21);
        assert!(!bus.pegc.is_256_color_active());
    }

    #[test]
    fn pegc_flat_access_fff00000_mirror() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        bus.a20_enabled = true;

        bus.write_word(0xE0102, 0x0001);
        assert!(bus.pegc.is_upper_vram_enabled());

        bus.write_byte(0xFFF00000, 0xDD);
        assert_eq!(bus.memory.state.pegc_vram.as_ref().unwrap()[0], 0xDD);
        assert_eq!(bus.read_byte(0xFFF00000), 0xDD);
        assert_eq!(bus.read_byte(0xF00000), 0xDD);
    }

    #[test]
    fn pegc_flat_access_disabled_returns_0xff() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        bus.a20_enabled = true;

        bus.memory.state.pegc_vram.as_mut().unwrap()[0] = 0xCC;
        assert_eq!(bus.read_byte(0xF00000), 0xFF);
        assert_eq!(bus.read_byte(0xFFF00000), 0xFF);
    }

    #[test]
    fn pegc_port_09a0_readback_256_color_status() {
        let mut bus = create_pc9821_bus();

        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 0);

        enable_pegc(&mut bus);
        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 1);

        disable_pegc(&mut bus);
        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 0);
    }

    #[test]
    fn pegc_port_09a0_readback_screen_mode() {
        let mut bus = create_pc9821_bus();

        bus.io_write_byte(0x09A0, 0x0D);
        assert_eq!(bus.io_read_byte(0x09A0), 0);

        bus.io_write_byte(0x6A, 0x69);
        bus.io_write_byte(0x09A0, 0x0D);
        assert_eq!(bus.io_read_byte(0x09A0), 1);

        bus.io_write_byte(0x6A, 0x68);
        bus.io_write_byte(0x09A0, 0x0D);
        assert_eq!(bus.io_read_byte(0x09A0), 0);
    }

    #[test]
    fn pegc_plane_mode_drawing_page_offset() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);
        bus.write_byte(0xE0100, 0x01);

        bus.pegc.state.rop_register = 0x0100;
        bus.pegc.state.write_mask = 0xFFFF;
        bus.pegc.state.block_length = 0x0FFF;
        bus.pegc.state.data_select = 1;

        bus.display_control.write_access_page(1);

        bus.write_word(0xA8000, 0xFFFF);

        let vram = bus.memory.state.pegc_vram.as_ref().unwrap();
        assert_ne!(
            vram[0x40000], 0,
            "page 1 at offset 0x40000 should be written"
        );
        assert_eq!(vram[0], 0, "page 0 at offset 0 should be untouched");
    }

    #[test]
    fn pegc_mmio_word_write_pattern_register() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.write_word(0xE0120, 0xBEEF);

        assert_eq!(bus.pegc.state.pattern_data[0], 0xEF);
        assert_eq!(bus.pegc.state.pattern_data[1], 0xBE);
    }

    #[test]
    fn pegc_mmio_word_read_pattern_register() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.pegc.state.pattern_data[0] = 0xEF;
        bus.pegc.state.pattern_data[1] = 0xBE;

        let value = bus.read_word(0xE0120);
        assert_eq!(value, 0xBEEF);
    }

    #[test]
    fn pegc_mmio_word_write_mode_register() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.write_word(0xE0100, 0x0001);
        assert!(bus.pegc.is_plane_mode());

        bus.write_word(0xE0100, 0x0000);
        assert!(bus.pegc.is_packed_pixel_mode());
    }

    #[test]
    fn pegc_port_09a0_readback_vram_access_mode() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.io_write_byte(0x09A0, 0x0B);
        assert_eq!(bus.io_read_byte(0x09A0), 1);

        bus.write_word(0xE0100, 0x0001);
        bus.io_write_byte(0x09A0, 0x0B);
        assert_eq!(bus.io_read_byte(0x09A0), 0);

        bus.write_word(0xE0100, 0x0000);
        bus.io_write_byte(0x09A0, 0x0B);
        assert_eq!(bus.io_read_byte(0x09A0), 1);
    }

    #[test]
    fn pegc_port_a4_blocked_in_one_screen_mode() {
        let mut bus = create_pc9821_bus();

        bus.io_write_byte(0xA4, 1);
        assert_eq!(bus.display_control.state.display_page, 1);

        bus.io_write_byte(0x6A, 0x69);
        bus.io_write_byte(0xA4, 0);
        assert_eq!(
            bus.display_control.state.display_page, 1,
            "write should be blocked in OneScreen mode"
        );

        bus.io_write_byte(0x6A, 0x68);
        bus.io_write_byte(0xA4, 0);
        assert_eq!(
            bus.display_control.state.display_page, 0,
            "write should succeed in TwoScreen mode"
        );
    }

    #[test]
    fn pegc_port_09a0_includes_gdc_clock2_bit() {
        let mut bus = create_pc9821_bus();
        enable_pegc(&mut bus);

        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 0x01, "PEGC active, no clock");

        bus.io_write_byte(0x6A, 0x85);
        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 0x03, "PEGC active + GDC CLOCK-2");

        bus.io_write_byte(0x6A, 0x84);
        bus.io_write_byte(0x09A0, 0x0A);
        assert_eq!(bus.io_read_byte(0x09A0), 0x01, "clock cleared, PEGC only");

        bus.io_write_byte(0x6A, 0x85);
        bus.io_write_byte(0x09A0, 0x04);
        let result = bus.io_read_byte(0x09A0);
        assert_eq!(
            result & 0x02,
            0x02,
            "GDC CLOCK-2 bit ORed into index 0x04 readback"
        );
    }
}
