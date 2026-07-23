//! IBM PC/AT (CS4031, i486DX2) machine for DOS/V.
//!
//! This crate models a period-correct PC/AT clone built on the Chips &
//! Technologies CS4031 chipset with a Tseng ET4000AX SVGA card.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

mod bus;
mod cmos;
mod config;
mod machine;
mod memory;
mod rom;
mod scheduler;

pub use bus::{
    AT_KEY_CURSOR_DOWN, AT_KEY_CURSOR_LEFT, AT_KEY_CURSOR_RIGHT, AT_KEY_CURSOR_UP, AT_KEY_DELETE,
    AT_KEY_END, AT_KEY_HOME, AT_KEY_INSERT, AT_KEY_KEYPAD_DIVIDE, AT_KEY_KEYPAD_ENTER,
    AT_KEY_PAGE_DOWN, AT_KEY_PAGE_UP, AT_KEY_RIGHT_ALT, AT_KEY_RIGHT_CTRL, AtBus,
};
pub use config::{AtBootDevice, AtModel, ClockConfig, PIT_CLOCK_HZ};
pub use machine::{AtMachine, build_automated_machine, build_untraced_machine};
pub use rom::{LoadedRoms, RomError, load_rom_set};

/// Read-only PC/AT hardware inspection view.
///
/// This type is for focused hardware assertions and cannot be restored.
#[derive(Debug, Clone)]
pub struct AtInspectionState {
    /// Cascaded PIC pair snapshot.
    pub pic: device::i8259a_pic::I8259aPicState,
    /// PIT snapshot.
    pub pit: device::i8253_pit::I8253PitState,
    /// Dual-8237 DMA snapshot.
    pub dma: device::at_dma::AtDmaState,
    /// 8042 keyboard controller snapshot.
    pub kbc: device::i8042_kbc::I8042KbcState,
    /// CS4031 chipset snapshot.
    pub chipset: device::cs4031::Cs4031State,
    /// RTC and CMOS RAM snapshot.
    pub rtc: device::mc146818_rtc::Mc146818RtcState,
    /// Keyboard LED state the last 0xED command programmed.
    pub keyboard_leds: u8,
    /// Whether the A20 gate is enabled.
    pub a20_enabled: bool,
}
