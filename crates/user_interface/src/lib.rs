//! The user interface for the emulator Neetan.
//!
//! Includes the following user facing UI through the "neetan" binary:
//! - Game manager
//! - Main emulator UI
//! - Copy command
//! - Create FDD / HDD command
//! - Convert HDD command

#![deny(unsafe_code)]

pub use common::Error;

use crate::{
    config::{EmulatorConfig, GuiResources, ManagerStart},
    input::KeyOverrides,
    manager::Manager,
    shell::{Shell, ShellOptions},
};

pub mod catalog;
pub mod config;
pub mod convert;
pub mod copy;
pub mod create;
mod editor;
mod file_browser;
mod gui_host;
#[cfg(test)]
mod gui_test_support;
mod host;
mod input;
mod language;
mod machines;
mod manager;
mod media_dialog;
pub mod rom_folders;
mod session;
mod shell;
pub mod texts;

/// The global allocator of the test binary. It counts the allocations of each thread.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: common::allocation_counter::CountingAllocator =
    common::allocation_counter::CountingAllocator;

/// The company name of the SDL pref path.
const COMPANY_NAME: &str = "neetan";
/// The name of the application.
pub const GAME_NAME: &str = "neetan";
/// The name of the application in the user interface.
pub(crate) const APPLICATION_TITLE: &str = "Neetan";
/// The version of the application.
pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The result type of the user interface.
pub type Result<T> = common::Result<T>;

/// The word "on" or "off" for `enabled`.
pub(crate) const fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

/// Opens the window and runs the game of `config` until the user quits.
/// The GUI font is the first file of the font paths of `gui_resources` that
/// exists, and the language files lie in its language folders.
pub fn run(
    config: EmulatorConfig,
    key_overrides: KeyOverrides,
    gui_resources: GuiResources,
) -> Result<()> {
    let mut shell = Shell::new(ShellOptions::for_config(&config, gui_resources))?;
    shell.start_session(config, key_overrides)?;
    shell.run()
}

/// Opens the window with the game manager of `start` and runs it and the
/// games it starts until the user quits.
pub fn run_manager(start: ManagerStart) -> Result<()> {
    let manager = Manager::new(start)?;
    let mut shell = Shell::new(ShellOptions::for_manager(manager.host()))?;
    shell.open_manager(manager)?;
    shell.run()
}
