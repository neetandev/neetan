#![forbid(unsafe_code)]

use common::{
    error, info,
    log::{Level, initialize_logger},
};
use neetan::{CARGO_PKG_VERSION, GAME_NAME};

#[cfg(debug_assertions)]
const DEFAULT_LOG_LEVEL: Level = Level::Debug;

#[cfg(not(debug_assertions))]
const DEFAULT_LOG_LEVEL: Level = Level::Info;

fn main() {
    initialize_logger(DEFAULT_LOG_LEVEL, vec![]);

    let action = match neetan::config::parse_args() {
        Ok(action) => action,
        Err(error) => {
            error!("{error:#}");
            std::process::exit(1);
        }
    };

    match action {
        neetan::config::Action::Run(config) => {
            info!("{GAME_NAME}");
            info!("Build version: {CARGO_PKG_VERSION}");

            if let Err(error) = neetan::run(*config) {
                error!("Error while executing the emulator: {error:#}");
                std::process::exit(1);
            }
        }
        neetan::config::Action::CreateFdd { path, fdd_type } => {
            if let Err(error) = neetan::create::create_fdd_image(&path, fdd_type) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        neetan::config::Action::CreateHdd { path, hdd_type } => {
            if let Err(error) = neetan::create::create_hdd_image(&path, hdd_type) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        neetan::config::Action::ConvertHdd { input, output } => {
            if let Err(error) = neetan::convert::convert_hdd_image(&input, &output) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
        neetan::config::Action::Copy { source, dest } => {
            if let Err(error) = neetan::copy::copy(source, dest) {
                error!("{error:#}");
                std::process::exit(1);
            }
        }
    }
}
