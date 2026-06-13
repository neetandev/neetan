//! Hardware device emulations for PC-98 peripherals.

#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod beeper;
pub mod bios;
pub mod cd_audio;
pub mod cdrom;
pub mod cdrom_pc88;
pub mod cgrom;
pub mod disk;
pub mod disk_backend;
pub mod disk_hle;
pub mod display_control;
pub mod egc;
pub mod fdd320_ppi;
pub mod fdd640k_hle;
pub mod floppy;
pub mod ga1280a;
pub mod grcg;
pub mod i8214_pic;
pub mod i8237_dma;
pub mod i8251_keyboard;
pub mod i8251_serial;
pub mod i8253_pit;
pub mod i8255;
pub mod i8255_mouse_ppi;
pub mod i8255_system_ppi;
pub mod i8257_dma;
pub mod i8259a_pic;
pub mod ide;
pub mod mpu_pc98ii;
#[cfg(feature = "mt32")]
pub mod mt32;
pub mod opn_fm;
pub mod palette;
pub mod palette_pc88;
pub mod pegc;
pub mod printer;
pub mod sasi;
#[cfg(feature = "sc55")]
pub mod sc55;
pub mod sdip;
pub mod sound_blaster_16;
pub mod soundboard_14;
pub mod soundboard_26k;
pub mod soundboard_86;
pub mod soundboard_ii;
pub mod upd3301_crtc;
pub mod upd4990a_rtc;
pub mod upd52611_crtc;
pub mod upd7220_gdc;
pub mod upd765a_fdc;
