//! The SDL3 start-up of the shell: the subsystems, the window and the
//! graphics backend.

use common::{Context, info, warn};
use sdl3::{
    Sdl,
    audio::AudioSubsystem,
    gamepad::GamepadSubsystem,
    video::{VideoSubsystem, Window, WindowBuilder},
};
use sdl3_backend::{DisplayAspectMode, GraphicsEngine, LegacySdlBackend, ModernSdlGpuBackend};

use crate::{
    GAME_NAME, Result,
    config::Backend,
    host::{MINIMUM_WINDOW_HEIGHT, MINIMUM_WINDOW_WIDTH},
};

/// The SDL3 context and its subsystems.
pub(super) struct SdlSubsystems {
    /// The SDL3 context.
    pub(super) context: Sdl,
    /// The audio subsystem.
    pub(super) audio: AudioSubsystem,
    /// The video subsystem.
    pub(super) video: VideoSubsystem,
    /// The gamepad subsystem, if it initialized.
    pub(super) gamepad: Option<GamepadSubsystem>,
}

/// Initializes SDL3, routes its log into the emulator log and opens the
/// subsystems.
pub(super) fn initialize_sdl3() -> Result<SdlSubsystems> {
    let context = sdl3::init().context("Failed to initialize SDL3")?;

    sdl3::log::set_log_priorities(sdl3::log::LogPriority::Verbose);
    sdl3::log::set_log_output_function(sdl3_log_callback);

    let audio = context
        .audio()
        .context("Failed to initialize SDL3 audio subsystem")?;

    let video = context
        .video()
        .context("Failed to initialize SDL3 video subsystem")?;

    // The gamepad subsystem is optional: a missing controller layer must not
    // prevent the emulator from starting.
    let gamepad = match context.gamepad() {
        Ok(subsystem) => Some(subsystem),
        Err(error) => {
            warn!("Failed to initialize SDL3 gamepad subsystem: {error}");
            None
        }
    };

    Ok(SdlSubsystems {
        context,
        audio,
        video,
        gamepad,
    })
}

/// Passes one SDL3 log message to the emulator log.
fn sdl3_log_callback(_category: i32, priority: sdl3::log::LogPriority, message: &str) {
    let level = match priority {
        sdl3::log::LogPriority::Trace | sdl3::log::LogPriority::Verbose => {
            common::log::Level::Trace
        }
        sdl3::log::LogPriority::Debug => common::log::Level::Debug,
        sdl3::log::LogPriority::Info => common::log::Level::Info,
        sdl3::log::LogPriority::Warn => common::log::Level::Warn,
        sdl3::log::LogPriority::Error | sdl3::log::LogPriority::Critical => {
            common::log::Level::Error
        }
    };
    common::log::log_record(level, "sdl3", format_args!("{message}"));
}

/// Logs the SDL3 version and the host platform, CPU count and memory.
pub(super) fn print_system_info() {
    let (sdl3_major, sdl3_minor, sdl3_patch) = sdl3::info::version();
    let sdl3_revision = sdl3::info::revision();
    info!("SDL3 v{sdl3_major}.{sdl3_minor}.{sdl3_patch} ({sdl3_revision})");
    let platform = sdl3::info::platform();
    info!("Running on {platform}");
    let cpu = sdl3::info::num_logical_cpu_cores();
    info!("System has {cpu} CPU(s)");
    let system_ram_mib = sdl3::info::system_ram();
    info!("System has {system_ram_mib} MiB");
}

/// Builds the hidden, resizable and centered window with the minimum size.
pub(super) fn build_window(
    video_subsystem: &VideoSubsystem,
    initial_width: u32,
    initial_height: u32,
    fullscreen: bool,
) -> Result<Window> {
    let mut builder: WindowBuilder = video_subsystem
        .window(GAME_NAME, initial_width, initial_height)
        .high_pixel_density()
        .resizable()
        .position_centered()
        .hidden();

    if fullscreen {
        builder = builder.fullscreen();
    }

    let window = builder
        .build()
        .context("Failed to create window with SDL3")?;
    if let Err(error) = window.set_minimum_size(MINIMUM_WINDOW_WIDTH, MINIMUM_WINDOW_HEIGHT) {
        warn!(
            "Failed to set the minimum window size {MINIMUM_WINDOW_WIDTH} x {MINIMUM_WINDOW_HEIGHT}: {error}"
        );
    }

    Ok(window)
}

/// Constructs the rendering backend and initializes its rendering surface.
///
/// When the modern SDL3 GPU API backend is requested, falls back to the legacy
/// SDL 2D Renderer automatically if either constructing the backend or
/// initializing its surface fails. Returns the engine plus the backend
/// actually selected.
pub(super) fn select_graphics_backend(
    aspect_mode: DisplayAspectMode,
    backend: Backend,
    large_native_target: bool,
    window: &mut Window,
) -> Result<(Box<dyn GraphicsEngine>, Backend)> {
    match backend {
        Backend::Legacy => {
            info!("Using legacy backend");
            let mut engine = LegacySdlBackend::new(aspect_mode, large_native_target);
            engine
                .on_resume(window, true)
                .context("Failed to initialize legacy SDL backend")?;
            Ok((Box::new(engine), Backend::Legacy))
        }
        Backend::Modern => {
            let modern_result = ModernSdlGpuBackend::new(aspect_mode, large_native_target)
                .and_then(|mut engine| engine.on_resume(window, true).map(|()| engine));

            match modern_result {
                Ok(engine) => {
                    info!("Using modern backend");
                    Ok((Box::new(engine), Backend::Modern))
                }
                Err(error) => {
                    warn!("Modern backend unavailable, falling back to legacy backend: {error:#}");
                    let mut legacy = LegacySdlBackend::new(aspect_mode, large_native_target);
                    legacy
                        .on_resume(window, true)
                        .context("Failed to initialize legacy backend after fallback")?;
                    Ok((Box::new(legacy), Backend::Legacy))
                }
            }
        }
    }
}
