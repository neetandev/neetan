//! The user interface for the emulator Neetan.
//!
//! Includes the following user facing UI through the "neetan" binary:
//! - Main emulator UI
//! - Copy command
//! - Create FDD / HDD command
//! - Convert HDD command

#![deny(unsafe_code)]

pub use common::Error;

use crate::{
    config::EmulatorConfig,
    input::KeyOverrides,
    shell::{Shell, ShellOptions},
};

pub mod catalog;
pub mod config;
pub mod convert;
pub mod copy;
pub mod create;
mod gui_host;
mod host;
mod input;
mod machines;
mod media_dialog;
pub mod rom_folders;
mod session;
mod shell;

/// The company name of the SDL pref path.
const COMPANY_NAME: &str = "neetan";
/// The name of the application.
pub const GAME_NAME: &str = "neetan";
/// The version of the application.
pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The result type of the user interface.
pub type Result<T> = common::Result<T>;

/// The word "on" or "off" for `enabled`.
pub(crate) const fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

/// Opens the window and runs the game of `config` until the user quits.
pub fn run(config: EmulatorConfig, key_overrides: KeyOverrides) -> Result<()> {
    let mut shell = Shell::new(ShellOptions::for_config(&config))?;
    shell.start_session(config, key_overrides)?;
    shell.run()
}
