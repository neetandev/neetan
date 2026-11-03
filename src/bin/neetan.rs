//! The `neetan` binary entry point.

#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(unsafe_code)]

use common::{
    error, info,
    log::{Level, initialize_logger},
};
use user_interface::{CARGO_PKG_VERSION, GAME_NAME};

#[cfg(debug_assertions)]
const DEFAULT_LOG_LEVEL: Level = Level::Debug;

#[cfg(not(debug_assertions))]
const DEFAULT_LOG_LEVEL: Level = Level::Info;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_console {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }

    /// Attaches the process to the console of its parent process, if the parent has one.
    ///
    /// The binary uses the Windows GUI subsystem and starts without a console. A start from a
    /// terminal keeps the log and the output of the disk image tools visible in that terminal.
    pub fn attach_parent_console() {
        // SAFETY: AttachConsole takes a plain process id and has no memory safety requirements.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    #[cfg(windows)]
    windows_console::attach_parent_console();

    initialize_logger(DEFAULT_LOG_LEVEL, vec![]);

    let action = match user_interface::config::parse_args() {
        Ok(action) => action,
        Err(error) => {
            error!("{error:#}");
            std::process::exit(1);
        }
    };

    match action {
        user_interface::config::Action::Run(config, key_overrides, gui_resources) => {
            info!("{GAME_NAME}");
            info!("Build version: {CARGO_PKG_VERSION}");

            if let Err(error) = user_interface::run(*config, key_overrides, gui_resources) {
                error!("Error while executing the emulator: {error:#}");
                std::process::exit(1);
            }
        }
        user_interface::config::Action::Manager(start) => {
            info!("{GAME_NAME}");
            info!("Build version: {CARGO_PKG_VERSION}");

            if let Err(error) = user_interface::run_manager(*start) {
                error!("Error while executing the game manager: {error:#}");
                std::process::exit(1);
            }
        }
        user_interface::config::Action::CreateFdd { path, fdd_type } => {
            if let Err(error) = user_interface::create::create_fdd_image(&path, fdd_type) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        user_interface::config::Action::CreateHdd { path, hdd_type } => {
            if let Err(error) = user_interface::create::create_hdd_image(&path, hdd_type) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        user_interface::config::Action::ConvertHdd { input, output } => {
            if let Err(error) = user_interface::convert::convert_hdd_image(&input, &output) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        user_interface::config::Action::Copy { source, dest } => {
            if let Err(error) = user_interface::copy::copy(source, dest) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
    }
}
