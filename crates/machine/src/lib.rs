//! PC-98 machine configurations.
//!
//! Contains the event-driven scheduler and memory-mapped bus
//! implementations for specific PC-98 models.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bus;
mod config;
mod machine;
mod memory;

use common::MachineModel;
pub use common::{NoTracing, OsBootStage, SchedulerState, Tracing};
use device::{
    beeper::BeeperState, cgrom::CgromState, display_control::DisplayControlState, egc::EgcState,
    fdd320_ppi::Fdd320PpiState, ga1280a::Ga1280aState, grcg::GrcgState,
    i8251_keyboard::I8251KeyboardState, i8251_serial::I8251SerialState, i8253_pit::I8253PitState,
    i8255_mouse_ppi::I8255MousePpiState, i8255_system_ppi::I8255SystemPpiState,
    i8259a_pic::I8259aPicState, palette::PaletteState, printer::PrinterState,
    sound_blaster_16::SoundBlaster16State, soundboard_14::Soundboard14State,
    soundboard_26k::Soundboard26kState, soundboard_86::Soundboard86State,
    upd765a_fdc::Upd765aFdcState, upd7220_gdc::GdcState, upd52611_crtc::Upd52611CrtcState,
};
pub use machine::{Machine, Pc9801F, Pc9801Ra, Pc9801Vm, Pc9801Vx, Pc9821Ap, Pc9821As};

pub use crate::{
    bus::{BootDevice, Pc9801Bus},
    config::ClockConfig,
    memory::Pc9801MemoryState,
};

/// CPU state snapshot, discriminated by CPU type.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuState {
    /// 8086 CPU state.
    I8086(cpu::I8086State),
    /// V30 CPU state.
    V30(cpu::V30State),
    /// 80286 CPU state.
    I286(cpu::I286State),
    /// 80386 CPU state.
    I386(cpu::I386State),
}

/// Complete machine state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineState {
    /// CPU register snapshot.
    pub cpu: CpuState,
    /// Machine model.
    pub machine_model: MachineModel,
    /// Memory subsystem snapshot.
    pub memory: Pc9801MemoryState,
    /// Clock configuration.
    pub clocks: ClockConfig,
    /// PIC snapshot.
    pub pic: I8259aPicState,
    /// Scheduler snapshot.
    pub scheduler: SchedulerState,
    /// PIT snapshot.
    pub pit: I8253PitState,
    /// Master GDC (text) snapshot.
    pub gdc_master: GdcState,
    /// Slave GDC (graphics) snapshot.
    pub gdc_slave: GdcState,
    /// Current CPU cycle count.
    pub current_cycle: u64,
    /// Cycle at which the next event fires.
    pub next_event_cycle: u64,
    /// Whether NMI is enabled.
    pub nmi_enabled: bool,
    /// Keyboard controller snapshot.
    pub keyboard: I8251KeyboardState,
    /// Serial controller (RS-232C i8251) snapshot.
    pub serial: I8251SerialState,
    /// A20 gate state.
    pub a20_enabled: bool,
    /// FDC µPD765A - 1MB floppy interface.
    pub fdc_1mb: Upd765aFdcState,
    /// FDC µPD765A - 640KB floppy interface.
    pub fdc_640k: Upd765aFdcState,
    /// Dual-mode FDC interface control.
    pub fdc_media: u8,
    /// PC-9801 320KB FDD PPI snapshot.
    pub fdd320_ppi: Fdd320PpiState,
    /// VRAM/EMS bank register.
    pub vram_ems_bank: u8,
    /// RAM window register.
    pub ram_window: u8,
    /// System PPI (i8255) snapshot.
    pub system_ppi: I8255SystemPpiState,
    /// Printer device snapshot.
    pub printer: PrinterState,
    /// CGROM controller snapshot.
    pub cgrom: CgromState,
    /// GRCG snapshot.
    pub grcg: GrcgState,
    /// EGC snapshot.
    pub egc: EgcState,
    /// Display control registers snapshot.
    pub display_control: DisplayControlState,
    /// CRTC uPD52611 snapshot.
    pub crtc: Upd52611CrtcState,
    /// Palette snapshot.
    pub palette: PaletteState,
    /// PC-9801-14 Music Generator (TMS3631) snapshot, if installed.
    pub soundboard_14: Option<Soundboard14State>,
    /// PC-9801-26K sound board (YM2203 OPN) snapshot, if installed.
    pub soundboard_26k: Option<Soundboard26kState>,
    /// PC-9801-86 sound board (YM2608 OPNA + PCM86) snapshot, if installed.
    pub soundboard_86: Option<Soundboard86State>,
    /// Sound Blaster 16 (CT2720) snapshot, if installed.
    pub sound_blaster_16: Option<SoundBlaster16State>,
    /// I-O DATA GA-1280A graphics accelerator snapshot, if installed.
    pub ga1280a: Option<Ga1280aState>,
    /// Beeper device snapshot.
    pub beeper: BeeperState,
    /// Mouse PPI (i8255) snapshot.
    pub mouse_ppi: I8255MousePpiState,
    /// Mouse interrupt timer setting register (port 0xBFDB).
    pub mouse_timer_setting: u8,
    /// 15M hole control register (port 0x043B).
    pub hole_15m_control: u8,
    /// Protected memory registration register (port 0x0567).
    pub protected_memory_max: u8,
    /// Whether NEC B-bank EMS (port 0x043F bit 1) is supported.
    pub b_bank_ems: bool,
    /// Current text RAM access wait penalty in CPU cycles.
    pub tram_wait: i64,
    /// Current graphics VRAM access wait penalty in CPU cycles.
    pub vram_wait: i64,
    /// Current GRCG VRAM access wait penalty in CPU cycles.
    pub grcg_wait: i64,
    /// Whether the BIOS interval timer single-shot service is currently armed.
    pub bios_interval_timer_active: bool,
}
