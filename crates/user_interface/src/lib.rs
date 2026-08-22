//! The user interface for the emulator Neetan.
//!
//! Includes the following user facing UI through the "neetan" binary:
//! - Main emulator UI
//! - Copy command
//! - Create FDD / HDD command
//! - Convert HDD command

#![deny(unsafe_code)]

use std::{
    path::Path,
    time::{Duration, Instant},
};

use audio_engine::AudioEngine;
pub use common::Error;
use common::{Context, JoystickState, KeyModifiers, Machine, StringError, error, info, warn};
use sdl3::{
    Sdl,
    audio::AudioSubsystem,
    event::{DisplayEvent, Event, WindowEvent},
    gamepad::{Gamepad, GamepadAxis, GamepadButton, GamepadSubsystem},
    keyboard::Scancode,
    mouse::MouseButton,
    video::{VideoSubsystem, Window, WindowBuilder},
};
use sdl3_backend::{
    DisplayAspectMode, GraphicsEngine, LegacySdlBackend, ModernSdlGpuBackend, RenderInstructions,
    Scaling,
};

use crate::{
    config::{AspectMode, Backend, EmulatorConfig, ScalingMode, Target, WindowMode},
    image_selector::{ImageEntry, ImageSelector, MediaType},
    input::{JoystickKey, KeyOverrides, KeyboardForwardingState, host_key_from_scancode},
    machines::{initialize_machine, selector_font_rom_data},
};

pub mod config;
pub mod convert;
pub mod copy;
pub mod create;
mod image_selector;
mod input;
mod machines;

const COMPANY_NAME: &str = "neetan";
pub const GAME_NAME: &str = "neetan";
pub const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const INITIAL_WINDOW_WIDTH: u32 = 1280;
const MAX_AUDIO_STEPS: usize = 40;
const SAMPLE_RATE: f64 = audio_engine::SAMPLE_RATE as f64;
/// Emulation speed multiplier applied while fast forward is held.
const FAST_FORWARD_FACTOR: f64 = 8.0;
/// Upper bound on the wall-clock interval used to pace a fast-forward step,
/// preventing a large burst after a host stall.
const FAST_FORWARD_MAX_ELAPSED: Duration = Duration::from_millis(100);
/// Analog-stick magnitude past which a left-stick axis counts as a held direction.
const GAMEPAD_AXIS_DEADZONE: i16 = 16384;

pub type Result<T> = common::Result<T>;

pub fn run(config: EmulatorConfig, key_overrides: KeyOverrides) -> Result<()> {
    let aspect_mode = config.aspect_mode;
    let (initial_width, initial_height) = initial_window_size(aspect_mode);
    let aspect_ratio = aspect_ratio_for_mode(aspect_mode);
    let graphics_display_aspect_mode = graphics_display_aspect_mode(aspect_mode);

    let (sdl_context, audio_subsystem, video_subsystem, gamepad_subsystem) = initialize_sdl3()?;

    print_system_into();

    let mut window = build_window(
        &video_subsystem,
        initial_width,
        initial_height,
        config.window_mode == WindowMode::Fullscreen,
    )?;

    if config.window_mode != WindowMode::Fullscreen
        && let Err(error) = window.set_aspect_ratio(aspect_ratio)
    {
        warn!("Failed to lock window aspect ratio to {aspect_ratio}: {error}");
    }

    let (width, height) = window.size();

    let (graphics_engine, backend) =
        select_graphics_backend(graphics_display_aspect_mode, &config, &mut window)?;

    if backend == Backend::Legacy && config.crt {
        warn!("CRT filter is not supported by the legacy SDL backend");
    }

    let mut application = Application::new(
        config,
        key_overrides,
        audio_subsystem,
        gamepad_subsystem,
        &window,
        graphics_engine,
        backend,
        (width as f32, height as f32),
    )?;

    application
        .graphics_engine
        .set_scaling(graphics_scaling(application.scaling));

    window.show();

    let mut event_pump = sdl_context
        .event_pump()
        .context("Failed to get the SDL3 event pump")?;

    'running: loop {
        for event in event_pump.poll_iter() {
            if application.handle_event(&event, Some(&window)) {
                break 'running;
            }
        }

        let busy_start = Instant::now();
        application.run_emulation();
        application.machine.tick_text_extractor();
        application.busy_duration += busy_start.elapsed();

        if let Err(error) = application.render_frame(&window) {
            error!("Failed to render next frame: {error:#}");
        }

        let elapsed = application.window_title_last_update.elapsed();
        if elapsed >= Duration::from_secs(5) {
            let busy_percent = (application.busy_duration.as_secs_f64() / elapsed.as_secs_f64()
                * 100.0)
                .round()
                .min(100.0) as u32;
            window.set_title(&format!("{GAME_NAME} ({busy_percent}% CPU)"));
            application.busy_duration = Duration::ZERO;
            application.window_title_last_update = Instant::now();
        }

        if application.should_quit {
            break 'running;
        }
    }

    Ok(())
}

fn print_system_into() {
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

fn initialize_sdl3() -> Result<(
    Sdl,
    AudioSubsystem,
    VideoSubsystem,
    Option<GamepadSubsystem>,
)> {
    let sdl_context = sdl3::init().context("Failed to initialize SDL3")?;

    sdl3::log::set_log_priorities(sdl3::log::LogPriority::Verbose);
    sdl3::log::set_log_output_function(sdl3_log_callback);

    let audio_subsystem = sdl_context
        .audio()
        .context("Failed to initialize SDL3 audio subsystem")?;

    let video_subsystem = sdl_context
        .video()
        .context("Failed to initialize SDL3 video subsystem")?;

    // The gamepad subsystem is optional: a missing controller layer must not
    // prevent the emulator from starting.
    let gamepad_subsystem = match sdl_context.gamepad() {
        Ok(subsystem) => Some(subsystem),
        Err(error) => {
            warn!("Failed to initialize SDL3 gamepad subsystem: {error}");
            None
        }
    };

    Ok((
        sdl_context,
        audio_subsystem,
        video_subsystem,
        gamepad_subsystem,
    ))
}

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

fn initial_window_size(aspect_mode: AspectMode) -> (u32, u32) {
    let initial_height = match aspect_mode {
        AspectMode::Aspect4By3 => 960,
        AspectMode::Aspect1By1 => 800,
    };
    (INITIAL_WINDOW_WIDTH, initial_height)
}

fn aspect_ratio_for_mode(aspect_mode: AspectMode) -> f32 {
    match aspect_mode {
        AspectMode::Aspect4By3 => 4.0 / 3.0,
        AspectMode::Aspect1By1 => 16.0 / 10.0,
    }
}

fn graphics_display_aspect_mode(aspect_mode: AspectMode) -> DisplayAspectMode {
    match aspect_mode {
        AspectMode::Aspect4By3 => DisplayAspectMode::Aspect4By3,
        AspectMode::Aspect1By1 => DisplayAspectMode::Aspect1By1,
    }
}

fn graphics_scaling(mode: ScalingMode) -> Scaling {
    match mode {
        ScalingMode::Nearest => Scaling::Nearest,
        ScalingMode::Bilinear => Scaling::Bilinear,
        ScalingMode::Pixelart => Scaling::Pixelart,
    }
}

fn build_window(
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

    Ok(window)
}

/// Constructs the rendering backend and initializes its rendering surface.
///
/// When the modern SDL3 GPU API backend is requested, falls back to the legacy
/// SDL 2D Renderer automatically if either constructing the backend or
/// initializing its surface fails. Returns the engine plus the backend
/// actually selected.
fn select_graphics_backend(
    aspect_mode: DisplayAspectMode,
    config: &EmulatorConfig,
    window: &mut Window,
) -> Result<(Box<dyn GraphicsEngine>, Backend)> {
    let large_native_target = config.graphicboard != config::GraphicboardType::None
        || matches!(config.target, Target::Towns | Target::X68k | Target::At);
    match config.backend {
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

struct Application {
    /// The startup configuration.
    config: EmulatorConfig,
    /// The emulated machine.
    machine: Box<dyn Machine>,
    /// The single in-memory runtime snapshot.
    runtime_snapshot: Option<save_state::MachineStateBlob>,
    /// The graphics engine.
    graphics_engine: Box<dyn GraphicsEngine>,
    /// Audio engine which outputs using the SDL3 push-based stream. Drives emulation speed.
    audio_engine: AudioEngine,
    /// The speed of the CPU on cycles per second.
    cpu_hz: f64,
    /// Tracks CPU cycle overshoot from previous audio steps for precise timing.
    cycle_overshoot: u64,
    /// Current logical viewport size.
    logical_size: (f32, f32),
    /// Current display scale factor for UI scaling.
    scale_factor: f32,
    /// Whether we should quit.
    should_quit: bool,
    /// Accumulated mouse X delta since last frame sync (sub-pixel).
    mouse_dx: f32,
    /// Accumulated mouse Y delta since last frame sync (sub-pixel).
    mouse_dy: f32,
    /// Current mouse button state.
    mouse_left: bool,
    mouse_right: bool,
    mouse_middle: bool,
    /// Whether relative mouse mode is active (Right Ctrl + M toggles).
    mouse_captured: bool,
    /// SDL3 gamepad subsystem, if it initialized. Kept alive while running.
    gamepad_subsystem: Option<GamepadSubsystem>,
    /// The currently opened gamepad, if one is connected.
    gamepad: Option<Gamepad>,
    /// Joystick directions and triggers from the gamepad d-pad and face buttons.
    gamepad_buttons: JoystickState,
    /// Left analog stick position (range -32768 to 32767).
    gamepad_axis_x: i16,
    gamepad_axis_y: i16,
    /// Joystick state from the keyboard fallback (used when no gamepad is connected).
    keyboard_joystick: JoystickState,
    /// Per-scancode native key-code overrides from `key.*` config bindings.
    key_overrides: KeyOverrides,
    keyboard_forwarding_state: KeyboardForwardingState,
    /// Floppy disk image entries for drive 1.
    fdd1_entries: Vec<ImageEntry>,
    /// Current index into fdd1_entries, or `None` if no floppy is loaded.
    fdd1_index: Option<usize>,
    /// Floppy disk image entries for drive 2.
    fdd2_entries: Vec<ImageEntry>,
    /// Current index into fdd2_entries, or `None` if no floppy is loaded.
    fdd2_index: Option<usize>,
    /// CD-ROM disc image entries.
    cdrom_entries: Vec<ImageEntry>,
    /// Current index into cdrom_entries, or `None` if no disc is loaded.
    cdrom_index: Option<usize>,
    /// Active image selection screen, if open.
    image_selector: Option<ImageSelector>,
    /// Expanded PC-98 CGROM data used by the image selector overlay.
    selector_font_rom: Vec<u8>,
    /// Whether the CRT effect is enabled.
    crt_enabled: bool,
    /// Composite subcarrier phase select (0..3), cycled with Right Ctrl + P. Swaps the
    /// PC-6001 artifact-color pair; only used by the composite present path.
    composite_phase: u32,
    /// Scaling method of the native texture.
    scaling: ScalingMode,
    /// The active graphics backend.
    backend: Backend,
    /// Whether the window is currently in fullscreen mode.
    fullscreen: bool,
    /// Accumulated emulation busy time in the current measurement window.
    busy_duration: Duration,
    /// When the window title was last updated with CPU usage.
    window_title_last_update: Instant,
    /// Whether fast forward is currently active (held).
    fast_forward: bool,
    /// Wall-clock instant of the last fast-forward emulation step, used to pace
    /// emulation at a fixed multiple of real time.
    last_emulation_tick: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemovableMediaAction {
    EjectFloppy { drive: usize },
    SelectFloppy { drive: usize, index: usize },
    EjectCdRom,
    SelectCdRom { index: usize },
    EjectCassette,
    SelectCassette,
}

impl RemovableMediaAction {
    /// Returns the host-visible slot affected by this action.
    fn slot(self) -> save_state::MediaSlot {
        match self {
            Self::EjectFloppy { drive } | Self::SelectFloppy { drive, .. } => {
                save_state::MediaSlot::new(save_state::MediaKind::Floppy, drive as u32)
            }
            Self::EjectCdRom | Self::SelectCdRom { .. } => {
                save_state::MediaSlot::new(save_state::MediaKind::CdRom, 0)
            }
            Self::EjectCassette | Self::SelectCassette => {
                save_state::MediaSlot::new(save_state::MediaKind::Cassette, 0)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RemovableMediaSelection {
    floppy: [Option<usize>; 2],
    cdrom: Option<usize>,
    cassette: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovableMediaRollback {
    selection: RemovableMediaSelection,
    slots: Vec<save_state::MediaSlot>,
}

struct RemovableMediaTransaction<State> {
    actions: Vec<RemovableMediaAction>,
    previous_media: RemovableMediaRollback,
    previous_state: State,
}

/// Finds a configured image by its normalized logical path.
fn configured_media_index(
    entries: &[ImageEntry],
    expected_path: &save_state::MediaSourcePath,
) -> Option<usize> {
    entries
        .iter()
        .position(|entry| save_state::MediaSourcePath::from_path(&entry.path) == *expected_path)
}

/// Plans a complete removable-media reconciliation without changing the machine.
fn plan_removable_media_actions(
    mismatch: &save_state::MediaMismatch,
    fdd1_entries: &[ImageEntry],
    fdd2_entries: &[ImageEntry],
    cdrom_entries: &[ImageEntry],
    cassette_path: Option<&Path>,
) -> std::result::Result<Vec<RemovableMediaAction>, String> {
    let mut actions: Vec<RemovableMediaAction> = Vec::new();
    for entry in mismatch.entries() {
        let expected = entry.expected();
        let active = entry.active();
        let slot = expected.or(active).unwrap().slot;
        if expected.is_some_and(|binding| binding.slot != slot)
            || active.is_some_and(|binding| binding.slot != slot)
        {
            return Err("save state and active media use different logical slots".to_owned());
        }
        if actions.iter().any(|action| action.slot() == slot) {
            return Err(format!(
                "multiple media bindings refer to {:?} drive {}",
                slot.kind,
                slot.index + 1
            ));
        }

        let action = match slot.kind {
            save_state::MediaKind::Floppy => {
                let drive = slot.index as usize;
                let configured_entries = match drive {
                    0 => fdd1_entries,
                    1 => fdd2_entries,
                    _ => return Err(format!("floppy drive {} is not host-selectable", drive + 1)),
                };
                match expected {
                    Some(binding) => {
                        let expected_path = binding.source_path.as_ref().ok_or_else(|| {
                            format!("floppy drive {} has no configured source path", drive + 1)
                        })?;
                        let index = configured_media_index(configured_entries, expected_path)
                            .ok_or_else(|| {
                                format!(
                                    "expected floppy for drive {} is not configured: {}",
                                    drive + 1,
                                    expected_path
                                )
                            })?;
                        RemovableMediaAction::SelectFloppy { drive, index }
                    }
                    None => RemovableMediaAction::EjectFloppy { drive },
                }
            }
            save_state::MediaKind::CdRom => {
                if slot.index != 0 {
                    return Err(format!(
                        "CD-ROM drive {} is not host-selectable",
                        slot.index + 1
                    ));
                }
                match expected {
                    Some(binding) => {
                        let expected_path = binding
                            .source_path
                            .as_ref()
                            .ok_or_else(|| "CD-ROM has no configured source path".to_owned())?;
                        let index = configured_media_index(cdrom_entries, expected_path)
                            .ok_or_else(|| {
                                format!("expected CD-ROM is not configured: {expected_path}")
                            })?;
                        RemovableMediaAction::SelectCdRom { index }
                    }
                    None => RemovableMediaAction::EjectCdRom,
                }
            }
            save_state::MediaKind::HardDisk => {
                return Err(format!("hard disk drive {} differs", slot.index + 1));
            }
            save_state::MediaKind::Cassette => {
                if slot.index != 0 {
                    return Err(format!(
                        "cassette deck {} is not host-selectable",
                        slot.index + 1
                    ));
                }
                match expected {
                    Some(binding) => {
                        let expected_path = binding
                            .source_path
                            .as_ref()
                            .ok_or_else(|| "cassette has no configured source path".to_owned())?;
                        let configured_path = cassette_path
                            .ok_or_else(|| "expected cassette is not configured".to_owned())?;
                        if save_state::MediaSourcePath::from_path(configured_path) != *expected_path
                        {
                            return Err(format!(
                                "expected cassette is not configured: {expected_path}"
                            ));
                        }
                        RemovableMediaAction::SelectCassette
                    }
                    None => RemovableMediaAction::EjectCassette,
                }
            }
        };
        actions.push(action);
    }
    Ok(actions)
}

/// Applies media and machine state together, restoring both on failure.
fn apply_media_and_state_transactionally<
    Target,
    State,
    ApplyMedia,
    ApplyState,
    RestoreMedia,
    RestoreState,
>(
    target: &mut Target,
    transaction: RemovableMediaTransaction<State>,
    mut apply_media: ApplyMedia,
    apply_state: ApplyState,
    restore_media: RestoreMedia,
    restore_state: RestoreState,
) -> std::result::Result<(), String>
where
    ApplyMedia: FnMut(&mut Target, RemovableMediaAction) -> std::result::Result<(), String>,
    ApplyState: FnOnce(&mut Target) -> std::result::Result<(), String>,
    RestoreMedia: FnOnce(&mut Target, &RemovableMediaRollback) -> std::result::Result<(), String>,
    RestoreState: FnOnce(&mut Target, &State) -> std::result::Result<(), String>,
{
    let RemovableMediaTransaction {
        actions,
        previous_media,
        previous_state,
    } = transaction;
    for action in actions {
        if let Err(error) = apply_media(target, action) {
            return Err(rollback_media_and_state(
                target,
                &previous_media,
                &previous_state,
                error,
                restore_media,
                restore_state,
            ));
        }
    }
    if let Err(error) = apply_state(target) {
        return Err(rollback_media_and_state(
            target,
            &previous_media,
            &previous_state,
            error,
            restore_media,
            restore_state,
        ));
    }
    Ok(())
}

/// Restores the prior media and machine state and combines rollback failures.
fn rollback_media_and_state<Target, State, RestoreMedia, RestoreState>(
    target: &mut Target,
    previous_media: &RemovableMediaRollback,
    previous_state: &State,
    error: String,
    restore_media: RestoreMedia,
    restore_state: RestoreState,
) -> String
where
    RestoreMedia: FnOnce(&mut Target, &RemovableMediaRollback) -> std::result::Result<(), String>,
    RestoreState: FnOnce(&mut Target, &State) -> std::result::Result<(), String>,
{
    let media_error = restore_media(target, previous_media).err();
    let state_error = restore_state(target, previous_state).err();
    match (media_error, state_error) {
        (None, None) => error,
        (Some(rollback_error), None) => {
            format!("{error}; restoring the previous media also failed: {rollback_error}")
        }
        (None, Some(rollback_error)) => {
            format!("{error}; restoring the previous machine state also failed: {rollback_error}")
        }
        (Some(media_error), Some(state_error)) => format!(
            "{error}; restoring the previous media also failed: {media_error}; restoring the previous machine state also failed: {state_error}"
        ),
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        self.machine.flush_printer();
        self.machine.flush_cartridges();
        self.machine.flush_floppies();
        self.machine.flush_hdds();
    }
}

impl Application {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: EmulatorConfig,
        key_overrides: KeyOverrides,
        audio_subsystem: AudioSubsystem,
        gamepad_subsystem: Option<GamepadSubsystem>,
        window: &Window,
        graphics_engine: Box<dyn GraphicsEngine>,
        backend: Backend,
        logical_size: (f32, f32),
    ) -> Result<Self> {
        let audio_engine = AudioEngine::new(audio_subsystem, config.audio_volume)
            .context("Failed to initialize audio")?;

        let fdd1_entries: Vec<ImageEntry> =
            config.fdd1.iter().cloned().map(ImageEntry::new).collect();
        let fdd2_entries: Vec<ImageEntry> =
            config.fdd2.iter().cloned().map(ImageEntry::new).collect();
        let cdrom_entries: Vec<ImageEntry> =
            config.cdrom.iter().cloned().map(ImageEntry::new).collect();

        let mut machine = initialize_machine(
            &config,
            audio_engine::SAMPLE_RATE as u32,
            crate::machines::sdl_host_date_time_source(),
        )?;
        let selector_font_rom = selector_font_rom_data(&config, machine.as_ref());

        if config.enable_extractor {
            machine.install_text_extractor(Box::new(text_extractor::ClipboardExtractor::new()));
            info!("Text extractor enabled (clipboard sink)");
        }

        let mut fdd1_index = None;
        if let Some(entry) = fdd1_entries.first() {
            match machine.insert_floppy_from_path(0, &entry.path) {
                Ok(desc) => {
                    info!("Inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = fdd2_entries.first() {
            match machine.insert_floppy_from_path(1, &entry.path) {
                Ok(desc) => {
                    info!("Inserted FDD2: {desc} from {}", entry.path.display());
                    fdd2_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }
        let mut cdrom_index = None;
        if let Some(entry) = cdrom_entries.first() {
            match machine.insert_cdrom(&entry.path) {
                Ok(desc) => {
                    info!("Inserted CD-ROM: {desc} from {}", entry.path.display());
                    cdrom_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }

        let cpu_hz = machine.cpu_clock_hz();
        let crt_enabled = config.crt && backend == Backend::Modern;
        let composite_phase = config.pc60_composite_phase;
        let scaling = config.scaling;

        let scale_factor = window.display_scale();

        info!("Window created with scale factor: {scale_factor}");
        if backend == Backend::Modern {
            info!("CRT effect set to {}", on_off(crt_enabled));
        }
        info!("Scaling set to {scaling}");

        Ok(Self {
            machine,
            runtime_snapshot: None,
            audio_engine,
            cpu_hz,
            cycle_overshoot: 0,
            logical_size,
            scale_factor,
            graphics_engine,
            should_quit: false,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            gamepad_subsystem,
            gamepad: None,
            gamepad_buttons: JoystickState::default(),
            gamepad_axis_x: 0,
            gamepad_axis_y: 0,
            keyboard_joystick: JoystickState::default(),
            mouse_left: false,
            mouse_right: false,
            mouse_middle: false,
            mouse_captured: false,
            key_overrides,
            keyboard_forwarding_state: KeyboardForwardingState::new(),
            fdd1_entries,
            fdd1_index,
            fdd2_entries,
            fdd2_index,
            cdrom_entries,
            cdrom_index,
            image_selector: None,
            selector_font_rom,
            crt_enabled,
            composite_phase,
            scaling,
            backend,
            fullscreen: config.window_mode == WindowMode::Fullscreen,
            busy_duration: Duration::ZERO,
            window_title_last_update: Instant::now(),
            fast_forward: false,
            last_emulation_tick: Instant::now(),
            config,
        })
    }

    /// Handles the most important window and keyboard events.
    fn handle_event(&mut self, event: &Event, window: Option<&Window>) -> bool {
        match event {
            Event::Quit => {
                self.should_quit = true;
                return true;
            }
            Event::Window {
                win_event: WindowEvent::Resized(width, height),
                ..
            } => {
                // Resized is in logical unit.
                let width = *width as u32;
                let height = *height as u32;
                let logical_size = (width as f32, height as f32);
                self.logical_size = logical_size;
            }
            Event::Window {
                win_event: WindowEvent::PixelSizeChanged(width, height),
                ..
            } => {
                // PixelSizeChanged is in physical unit.
                let width = *width as u32;
                let height = *height as u32;
                let logical_size = (
                    width as f32 / self.scale_factor,
                    height as f32 / self.scale_factor,
                );
                self.logical_size = logical_size;
            }
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => {
                self.set_fast_forward(false);
                self.audio_engine.pause();
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } if self.image_selector.is_none() => {
                self.audio_engine.resume();
            }
            Event::Display {
                display_event: DisplayEvent::ContentScaleChanged,
                ..
            } => {
                if let Some(scale) = window.map(|w| w.display_scale()) {
                    self.scale_factor = scale;
                    info!("Scale factor changed to: {scale}");
                }
            }
            Event::KeyDown {
                scancode,
                keymod,
                repeat,
                ..
            } => {
                if self.image_selector.is_some() {
                    if !repeat {
                        self.handle_selector_key(*scancode, keymod.rctrl());
                    }
                } else {
                    let modifiers = KeyModifiers {
                        shift: keymod.shift(),
                        ctrl: keymod.ctrl(),
                    };
                    let host_key = (*scancode).and_then(host_key_from_scancode);
                    let native_override =
                        (*scancode).and_then(|scancode| self.key_overrides.get(scancode));
                    self.keyboard_forwarding_state.handle_key_down(
                        *scancode,
                        keymod.rctrl(),
                        host_key,
                        modifiers,
                        native_override,
                        *repeat,
                    );
                    self.keyboard_forwarding_state
                        .apply_pending_actions(self.machine.as_mut());

                    // Keyboard joystick fallback (only used when no gamepad is
                    // connected): drive the joystick directions/triggers.
                    if let Some(key) = JoystickKey::from_scancode(*scancode) {
                        key.apply(&mut self.keyboard_joystick, true);
                        self.update_joystick();
                    }

                    if !repeat && keymod.rctrl() && *scancode == Some(Scancode::M) {
                        if let Some(w) = window {
                            self.toggle_mouse_capture(w);
                        }
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::Q) {
                        self.should_quit = true;
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::Return) {
                        if let Some(w) = window {
                            if let Err(error) = w.set_fullscreen(!self.fullscreen) {
                                warn!("Failed to toggle fullscreen: {error}");
                            } else {
                                self.fullscreen = !self.fullscreen;
                            }
                        }
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::C) {
                        self.toggle_crt();
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::S) {
                        self.toggle_scaling();
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::P) {
                        self.cycle_composite_phase();
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::_5) {
                        self.quick_save();
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::_9) {
                        self.quick_load();
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::_1) {
                        self.open_or_toggle_selector(MediaType::Floppy(0));
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::_2) {
                        self.open_or_toggle_selector(MediaType::Floppy(1));
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::_3) {
                        self.open_or_toggle_selector(MediaType::CdRom);
                    } else if !repeat && keymod.rctrl() && *scancode == Some(Scancode::R) {
                        self.hard_reset();
                    } else if keymod.rctrl() && *scancode == Some(Scancode::F) {
                        self.set_fast_forward(true);
                    }
                }
            }
            Event::KeyUp {
                scancode, repeat, ..
            } => {
                if self.fast_forward
                    && matches!(scancode, Some(Scancode::F) | Some(Scancode::RCtrl))
                {
                    self.set_fast_forward(false);
                }
                if self.image_selector.is_none() {
                    self.keyboard_forwarding_state
                        .handle_key_up(*scancode, *repeat);
                    self.keyboard_forwarding_state
                        .apply_pending_actions(self.machine.as_mut());
                    if let Some(key) = JoystickKey::from_scancode(*scancode) {
                        key.apply(&mut self.keyboard_joystick, false);
                        self.update_joystick();
                    }
                }
            }
            Event::MouseMotion { xrel, yrel, .. } if self.mouse_captured => {
                self.mouse_dx += xrel;
                self.mouse_dy += yrel;
            }
            Event::MouseButtonDown { mouse_btn, .. } if self.mouse_captured => {
                match mouse_btn {
                    MouseButton::Left => self.mouse_left = true,
                    MouseButton::Right => self.mouse_right = true,
                    MouseButton::Middle => self.mouse_middle = true,
                    _ => {}
                }
                self.machine.set_mouse_buttons(
                    self.mouse_left,
                    self.mouse_right,
                    self.mouse_middle,
                );
            }
            Event::MouseButtonUp { mouse_btn, .. } if self.mouse_captured => {
                match mouse_btn {
                    MouseButton::Left => self.mouse_left = false,
                    MouseButton::Right => self.mouse_right = false,
                    MouseButton::Middle => self.mouse_middle = false,
                    _ => {}
                }
                self.machine.set_mouse_buttons(
                    self.mouse_left,
                    self.mouse_right,
                    self.mouse_middle,
                );
            }
            Event::GamepadAdded { which } => {
                self.open_gamepad(*which);
            }
            Event::GamepadRemoved { which } => {
                if self
                    .gamepad
                    .as_ref()
                    .is_some_and(|pad| pad.instance_id() == *which)
                {
                    self.gamepad = None;
                    self.gamepad_buttons = JoystickState::default();
                    self.gamepad_axis_x = 0;
                    self.gamepad_axis_y = 0;
                    self.update_joystick();
                }
            }
            Event::GamepadButtonDown { which, button } => {
                self.handle_gamepad_button(*which, *button, true);
            }
            Event::GamepadButtonUp { which, button } => {
                self.handle_gamepad_button(*which, *button, false);
            }
            Event::GamepadAxisMotion { which, axis, value } => {
                self.handle_gamepad_axis(*which, *axis, *value);
            }
            _ => {}
        }

        false
    }

    /// Opens a connected gamepad if one is not already in use.
    fn open_gamepad(&mut self, which: u32) {
        if self.gamepad.is_some() {
            return;
        }
        let Some(subsystem) = self.gamepad_subsystem.as_ref() else {
            return;
        };
        match subsystem.open(which) {
            Ok(pad) => {
                info!("Gamepad connected (instance {which})");
                self.gamepad = Some(pad);
            }
            Err(error) => warn!("Failed to open gamepad {which}: {error}"),
        }
    }

    /// Records a gamepad button change and updates the joystick port.
    fn handle_gamepad_button(&mut self, which: u32, button: GamepadButton, pressed: bool) {
        if self
            .gamepad
            .as_ref()
            .is_none_or(|pad| pad.instance_id() != which)
        {
            return;
        }
        match button {
            GamepadButton::DpadUp => self.gamepad_buttons.up = pressed,
            GamepadButton::DpadDown => self.gamepad_buttons.down = pressed,
            GamepadButton::DpadLeft => self.gamepad_buttons.left = pressed,
            GamepadButton::DpadRight => self.gamepad_buttons.right = pressed,
            // Face and shoulder buttons cover the 6-button pad (A/B/C/X/Y/Z);
            // machines with only two triggers use just A and B.
            GamepadButton::South => self.gamepad_buttons.trigger1 = pressed,
            GamepadButton::East => self.gamepad_buttons.trigger2 = pressed,
            GamepadButton::West => self.gamepad_buttons.button_c = pressed,
            GamepadButton::North => self.gamepad_buttons.button_x = pressed,
            GamepadButton::LeftShoulder => self.gamepad_buttons.button_y = pressed,
            GamepadButton::RightShoulder => self.gamepad_buttons.button_z = pressed,
            GamepadButton::Start => self.gamepad_buttons.run = pressed,
            GamepadButton::Back => self.gamepad_buttons.select = pressed,
            _ => return,
        }
        self.update_joystick();
    }

    /// Records a left-stick axis change and updates the joystick port.
    fn handle_gamepad_axis(&mut self, which: u32, axis: GamepadAxis, value: i16) {
        if self
            .gamepad
            .as_ref()
            .is_none_or(|pad| pad.instance_id() != which)
        {
            return;
        }
        match axis {
            GamepadAxis::LeftX => self.gamepad_axis_x = value,
            GamepadAxis::LeftY => self.gamepad_axis_y = value,
            _ => return,
        }
        self.update_joystick();
    }

    /// Recomputes the effective joystick state and pushes it to the machine.
    /// A connected gamepad takes precedence over the keyboard fallback.
    fn update_joystick(&mut self) {
        let connected = self.gamepad.is_some();
        let state = if connected {
            let mut state = self.gamepad_buttons;
            // Fold the left analog stick into the directions past a deadzone.
            state.left |= self.gamepad_axis_x <= -GAMEPAD_AXIS_DEADZONE;
            state.right |= self.gamepad_axis_x >= GAMEPAD_AXIS_DEADZONE;
            state.up |= self.gamepad_axis_y <= -GAMEPAD_AXIS_DEADZONE;
            state.down |= self.gamepad_axis_y >= GAMEPAD_AXIS_DEADZONE;
            state
        } else {
            self.keyboard_joystick
        };
        self.machine.set_joystick(0, state);
        let axes = connected.then_some((self.gamepad_axis_x, self.gamepad_axis_y));
        self.machine.set_joystick_axes(0, axes);
    }

    /// Toggles mouse capture (relative mouse mode) on the given window.
    fn toggle_mouse_capture(&mut self, window: &Window) {
        let desired = !self.mouse_captured;

        if let Err(error) = window.set_relative_mouse_mode(desired) {
            warn!("Failed to set relative mouse mode: {error}");
            return;
        }

        self.mouse_captured = desired;

        if !self.mouse_captured {
            // Release all buttons when uncapturing.
            self.mouse_left = false;
            self.mouse_right = false;
            self.mouse_middle = false;
            self.machine.set_mouse_buttons(false, false, false);
            self.mouse_dx = 0.0;
            self.mouse_dy = 0.0;
        }

        info!(
            "Mouse {}",
            if self.mouse_captured {
                "captured"
            } else {
                "released"
            }
        );
    }

    fn eject_floppy(&mut self, drive: usize) {
        self.machine.eject_floppy(drive);
        match drive {
            0 => self.fdd1_index = None,
            1 => self.fdd2_index = None,
            _ => {}
        }
        info!("Ejected FDD{}", drive + 1);
    }

    fn select_floppy(&mut self, drive: usize, index: usize) -> std::result::Result<(), String> {
        let entries = match drive {
            0 => &self.fdd1_entries,
            1 => &self.fdd2_entries,
            _ => return Err(format!("floppy drive {} is not available", drive + 1)),
        };

        if index >= entries.len() {
            return Err(format!("floppy image index {index} is not available"));
        }

        let path = &entries[index].path;
        match self.machine.insert_floppy_from_path(drive, path) {
            Ok(description) => {
                info!(
                    "Selected FDD{}: {description} from {}",
                    drive + 1,
                    path.display()
                );
            }
            Err(error) => {
                error!("Failed to select FDD{}: {error}", drive + 1);
                return Err(error);
            }
        }

        match drive {
            0 => self.fdd1_index = Some(index),
            1 => self.fdd2_index = Some(index),
            _ => {}
        }
        Ok(())
    }

    fn eject_cdrom(&mut self) {
        self.machine.eject_cdrom();
        self.cdrom_index = None;
        info!("Ejected CD-ROM");
    }

    fn select_cdrom(&mut self, index: usize) -> std::result::Result<(), String> {
        if index >= self.cdrom_entries.len() {
            return Err(format!("CD-ROM image index {index} is not available"));
        }

        let path = &self.cdrom_entries[index].path;
        match self.machine.insert_cdrom(path) {
            Ok(description) => {
                info!("Selected CD-ROM: {description} from {}", path.display());
            }
            Err(error) => {
                error!("Failed to select CD-ROM: {error}");
                return Err(error);
            }
        }

        self.cdrom_index = Some(index);
        Ok(())
    }

    fn eject_cassette(&mut self) {
        self.machine.eject_cassette();
        info!("Ejected cassette");
    }

    fn select_cassette(&mut self) -> std::result::Result<(), String> {
        let path = self
            .config
            .cassette
            .clone()
            .ok_or_else(|| "cassette image is not configured".to_owned())?;
        match self.machine.insert_cassette(&path) {
            Ok(description) => info!("Selected cassette: {description}"),
            Err(error) => {
                error!("Failed to select cassette: {error}");
                return Err(error);
            }
        }
        Ok(())
    }

    fn open_or_toggle_selector(&mut self, media_type: MediaType) {
        if let Some(ref selector) = self.image_selector
            && *selector.media_type() == media_type
        {
            self.close_selector();
            return;
        }

        let (entries, current_index) = match &media_type {
            MediaType::Floppy(0) => (&self.fdd1_entries, self.fdd1_index),
            MediaType::Floppy(_) => (&self.fdd2_entries, self.fdd2_index),
            MediaType::CdRom => (&self.cdrom_entries, self.cdrom_index),
        };

        // Display position: None (empty) -> 0, Some(n) -> n + 1.
        let display_cursor = current_index.map_or(0, |n| n + 1);
        let display_count = entries.len() + 1;

        if let Some(ref mut selector) = self.image_selector {
            selector.switch_media(media_type, display_cursor, display_count);
        } else {
            self.audio_engine.pause();
            self.image_selector = Some(ImageSelector::new(
                media_type,
                display_cursor,
                display_count,
                &self.selector_font_rom,
            ));
        }
    }

    fn close_selector(&mut self) {
        self.image_selector = None;
        self.audio_engine.resume();
    }

    fn toggle_crt(&mut self) {
        if self.backend == Backend::Legacy {
            return;
        }
        self.crt_enabled = !self.crt_enabled;
        info!("CRT effect set to {}", on_off(self.crt_enabled));
    }

    fn cycle_composite_phase(&mut self) {
        if self.config.target != Target::Pc60 {
            return;
        }
        self.composite_phase = (self.composite_phase + 1) % 4;
        info!("Composite artifact phase set to {}", self.composite_phase);
    }

    fn toggle_scaling(&mut self) {
        self.scaling = match self.scaling {
            ScalingMode::Nearest => ScalingMode::Bilinear,
            ScalingMode::Bilinear => ScalingMode::Pixelart,
            ScalingMode::Pixelart => ScalingMode::Nearest,
        };
        self.graphics_engine
            .set_scaling(graphics_scaling(self.scaling));
        info!("Scaling set to {}", self.scaling);
    }

    fn handle_selector_key(&mut self, scancode: Option<Scancode>, rctrl_held: bool) {
        let Some(code) = scancode else { return };

        match code {
            Scancode::Up => {
                if let Some(ref mut selector) = self.image_selector {
                    selector.move_up();
                }
            }
            Scancode::Down => {
                if let Some(ref mut selector) = self.image_selector {
                    let count = match selector.media_type() {
                        MediaType::Floppy(0) => self.fdd1_entries.len() + 1,
                        MediaType::Floppy(_) => self.fdd2_entries.len() + 1,
                        MediaType::CdRom => self.cdrom_entries.len() + 1,
                    };
                    selector.move_down(count);
                }
            }
            Scancode::Return | Scancode::KpEnter => {
                if let Some(ref selector) = self.image_selector {
                    let media_type = selector.media_type().clone();
                    let cursor = selector.cursor();
                    match &media_type {
                        MediaType::Floppy(drive) => {
                            if cursor == 0 {
                                self.eject_floppy(*drive);
                            } else {
                                let _ = self.select_floppy(*drive, cursor - 1);
                            }
                        }
                        MediaType::CdRom => {
                            if cursor == 0 {
                                self.eject_cdrom();
                            } else {
                                let _ = self.select_cdrom(cursor - 1);
                            }
                        }
                    }
                }
                self.close_selector();
            }
            Scancode::Escape => {
                self.close_selector();
            }
            Scancode::_1 if rctrl_held => {
                self.open_or_toggle_selector(MediaType::Floppy(0));
            }
            Scancode::_2 if rctrl_held => {
                self.open_or_toggle_selector(MediaType::Floppy(1));
            }
            Scancode::_3 if rctrl_held => {
                self.open_or_toggle_selector(MediaType::CdRom);
            }
            _ => {}
        }
    }

    fn hard_reset(&mut self) {
        self.machine.flush_printer();
        self.machine.flush_cartridges();
        self.machine.flush_floppies();
        self.machine.flush_hdds();

        let mut machine = match initialize_machine(
            &self.config,
            audio_engine::SAMPLE_RATE as u32,
            crate::machines::sdl_host_date_time_source(),
        ) {
            Ok(machine) => machine,
            Err(error) => {
                warn!("Hard reset failed to re-create the machine: {error:#}");
                return;
            }
        };

        if self.config.enable_extractor {
            machine.install_text_extractor(Box::new(text_extractor::ClipboardExtractor::new()));
        }

        let mut fdd1_index = None;
        if let Some(entry) = self.fdd1_entries.first() {
            match machine.insert_floppy_from_path(0, &entry.path) {
                Ok(desc) => {
                    info!("Re-inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert FDD1: {error}"),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = self.fdd2_entries.first() {
            match machine.insert_floppy_from_path(1, &entry.path) {
                Ok(desc) => {
                    info!("Re-inserted FDD2: {desc} from {}", entry.path.display());
                    fdd2_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert FDD2: {error}"),
            }
        }
        let mut cdrom_index = None;
        if let Some(entry) = self.cdrom_entries.first() {
            match machine.insert_cdrom(&entry.path) {
                Ok(desc) => {
                    info!("Re-inserted CD-ROM: {desc} from {}", entry.path.display());
                    cdrom_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert CD-ROM: {error}"),
            }
        }

        self.machine = machine;
        self.fdd1_index = fdd1_index;
        self.fdd2_index = fdd2_index;
        self.cdrom_index = cdrom_index;
        self.cpu_hz = self.machine.cpu_clock_hz();
        self.cycle_overshoot = 0;
        self.keyboard_forwarding_state = KeyboardForwardingState::new();
        self.audio_engine.reset_buffer();
        info!("Hard reset complete");
    }

    fn quick_save(&mut self) {
        match self.machine.capture_state() {
            Ok(snapshot) => {
                self.runtime_snapshot = Some(snapshot);
                info!("Quick save captured");
            }
            Err(error) => warn!("Quick save failed: {error}"),
        }
    }

    /// Returns the current host-selected removable media state.
    fn removable_media_selection(
        &self,
        mismatch: &save_state::MediaMismatch,
    ) -> RemovableMediaSelection {
        let cassette = mismatch.entries().iter().find_map(|entry| {
            let binding = entry.expected().or(entry.active())?;
            (binding.slot.kind == save_state::MediaKind::Cassette)
                .then_some(entry.active().is_some())
        });
        RemovableMediaSelection {
            floppy: [self.fdd1_index, self.fdd2_index],
            cdrom: self.cdrom_index,
            cassette,
        }
    }

    /// Applies one planned removable-media change.
    fn apply_removable_media_action(
        &mut self,
        action: RemovableMediaAction,
    ) -> std::result::Result<(), String> {
        match action {
            RemovableMediaAction::EjectFloppy { drive } => self.eject_floppy(drive),
            RemovableMediaAction::SelectFloppy { drive, index } => {
                self.select_floppy(drive, index)?;
            }
            RemovableMediaAction::EjectCdRom => self.eject_cdrom(),
            RemovableMediaAction::SelectCdRom { index } => {
                self.select_cdrom(index)?;
            }
            RemovableMediaAction::EjectCassette => self.eject_cassette(),
            RemovableMediaAction::SelectCassette => self.select_cassette()?,
        }
        Ok(())
    }

    /// Restores a previously recorded removable-media selection.
    fn restore_removable_media_selection(
        &mut self,
        rollback: &RemovableMediaRollback,
    ) -> std::result::Result<(), String> {
        let selection = rollback.selection;
        for drive in 0..2 {
            let current = match drive {
                0 => self.fdd1_index,
                1 => self.fdd2_index,
                _ => unreachable!(),
            };
            let slot = save_state::MediaSlot::new(save_state::MediaKind::Floppy, drive as u32);
            if current == selection.floppy[drive] && !rollback.slots.contains(&slot) {
                continue;
            }
            match selection.floppy[drive] {
                Some(index) => self.select_floppy(drive, index)?,
                None => self.eject_floppy(drive),
            }
        }
        let cdrom_slot = save_state::MediaSlot::new(save_state::MediaKind::CdRom, 0);
        if self.cdrom_index != selection.cdrom || rollback.slots.contains(&cdrom_slot) {
            match selection.cdrom {
                Some(index) => self.select_cdrom(index)?,
                None => self.eject_cdrom(),
            }
        }
        let cassette_slot = save_state::MediaSlot::new(save_state::MediaKind::Cassette, 0);
        if let Some(cassette_mounted) = selection.cassette
            && rollback.slots.contains(&cassette_slot)
        {
            if cassette_mounted {
                self.select_cassette()?;
            } else {
                self.eject_cassette();
            }
        }
        Ok(())
    }

    /// Plans snapshot media changes and captures their rollback selection.
    fn removable_media_transaction(
        &mut self,
        mismatch: &save_state::MediaMismatch,
    ) -> std::result::Result<(Vec<RemovableMediaAction>, RemovableMediaRollback), String> {
        let actions = plan_removable_media_actions(
            mismatch,
            &self.fdd1_entries,
            &self.fdd2_entries,
            &self.cdrom_entries,
            self.config.cassette.as_deref(),
        )?;
        let previous = RemovableMediaRollback {
            selection: self.removable_media_selection(mismatch),
            slots: actions.iter().map(|action| action.slot()).collect(),
        };
        Ok((actions, previous))
    }

    /// Restores one runtime snapshot, including its removable-media set.
    fn restore_runtime_snapshot(
        &mut self,
        snapshot: &save_state::MachineStateBlob,
    ) -> std::result::Result<(), save_state::SaveStateError> {
        let mismatch = match self.machine.restore_state(snapshot) {
            Err(save_state::SaveStateError::MediaMismatch(mismatch)) => mismatch,
            result => return result,
        };
        info!("Quick load is changing removable media: {mismatch}");
        let rollback_snapshot = self.machine.capture_state()?;
        let (actions, previous_media) =
            self.removable_media_transaction(&mismatch)
                .map_err(|error| {
                    save_state::SaveStateError::InvalidInvariant(format!(
                        "quick load media change failed: {mismatch}: {error}"
                    ))
                })?;
        apply_media_and_state_transactionally(
            self,
            RemovableMediaTransaction {
                actions,
                previous_media,
                previous_state: rollback_snapshot,
            },
            Application::apply_removable_media_action,
            |application| {
                application
                    .machine
                    .restore_state(snapshot)
                    .map_err(|error| error.to_string())
            },
            Application::restore_removable_media_selection,
            |application, rollback_snapshot| {
                application
                    .machine
                    .restore_state(rollback_snapshot)
                    .map_err(|error| error.to_string())
            },
        )
        .map_err(|error| {
            save_state::SaveStateError::InvalidInvariant(format!(
                "quick load media change failed: {mismatch}: {error}"
            ))
        })
    }

    fn quick_load(&mut self) {
        let Some(snapshot) = self.runtime_snapshot.take() else {
            warn!("Quick load ignored because no runtime snapshot exists");
            return;
        };
        let restore_result = self.restore_runtime_snapshot(&snapshot);
        self.runtime_snapshot = Some(snapshot);
        match restore_result {
            Ok(()) => {
                self.cycle_overshoot = 0;
                self.mouse_dx = 0.0;
                self.mouse_dy = 0.0;
                self.keyboard_forwarding_state = KeyboardForwardingState::new();
                self.machine.set_mouse_buttons(
                    self.mouse_left,
                    self.mouse_right,
                    self.mouse_middle,
                );
                self.update_joystick();
                self.last_emulation_tick = Instant::now();
                self.set_fast_forward(false);
                self.audio_engine.reset_buffer();
                self.audio_engine.resume();
                info!("Quick load restored");
            }
            Err(error) => warn!("Quick load failed: {error}"),
        }
    }

    fn set_fast_forward(&mut self, enabled: bool) {
        if enabled == self.fast_forward {
            return;
        }
        self.fast_forward = enabled;
        if enabled {
            self.last_emulation_tick = Instant::now();
            self.audio_engine.pause();
        } else {
            self.audio_engine.reset_buffer();
            self.audio_engine.resume();
        }
    }

    fn run_emulation(&mut self) {
        if self.image_selector.is_some() {
            return;
        }

        // Flush accumulated mouse movement into the emulated machine.
        if self.mouse_captured {
            let dx = self.mouse_dx.round() as i16;
            let dy = self.mouse_dy.round() as i16;
            self.machine.push_mouse_delta(dx, dy);
            self.mouse_dx = 0.0;
            self.mouse_dy = 0.0;
        }

        if self.fast_forward {
            self.run_emulation_fast_forward();
            return;
        }

        for _ in 0..MAX_AUDIO_STEPS {
            let needed_frames = self.audio_engine.frames_needed() as u64;
            if needed_frames == 0 {
                break;
            }
            let raw_cycles = (needed_frames as f64 * self.cpu_hz / SAMPLE_RATE).round() as u64;

            // If a previous step overshot by more cycles than this step needs,
            // consume only what's needed and carry the remainder to avoid timing drift.
            if self.cycle_overshoot >= raw_cycles {
                self.cycle_overshoot -= raw_cycles;
                self.audio_engine.push_samples(self.machine.as_mut());
                continue;
            }

            let cycles = raw_cycles - self.cycle_overshoot;
            self.cycle_overshoot = 0;

            let ran_cycles = self.machine.run_for(cycles);
            if ran_cycles > cycles {
                self.cycle_overshoot = ran_cycles - cycles;
            }
            self.audio_engine.push_samples(self.machine.as_mut());

            if self.machine.shutdown_requested() {
                info!("Guest triggered system shutdown");
                self.should_quit = true;
                return;
            }
        }
    }

    fn run_emulation_fast_forward(&mut self) {
        let elapsed = self
            .last_emulation_tick
            .elapsed()
            .min(FAST_FORWARD_MAX_ELAPSED);
        self.last_emulation_tick = Instant::now();

        let target_cycles = (elapsed.as_secs_f64() * self.cpu_hz * FAST_FORWARD_FACTOR) as u64;
        let chunk_cycles =
            (audio_engine::STEP_FRAMES as f64 * self.cpu_hz / SAMPLE_RATE).round() as u64;
        if chunk_cycles == 0 {
            return;
        }

        let mut ran = 0u64;
        while ran < target_cycles {
            self.machine.run_for(chunk_cycles);
            self.audio_engine.discard_samples(self.machine.as_mut());
            ran += chunk_cycles;

            if self.machine.shutdown_requested() {
                info!("Guest triggered system shutdown");
                self.should_quit = true;
                return;
            }
        }
    }

    fn render_frame(&mut self, window: &Window) -> Result<()> {
        let render_instructions = if let Some(ref mut selector) = self.image_selector {
            let (entries, loaded_index) = match selector.media_type() {
                MediaType::Floppy(0) => (&self.fdd1_entries, self.fdd1_index),
                MediaType::Floppy(_) => (&self.fdd2_entries, self.fdd2_index),
                MediaType::CdRom => (&self.cdrom_entries, self.cdrom_index),
            };
            selector.ensure_render(entries, loaded_index);
            RenderInstructions {
                framebuffer: selector.framebuffer(),
                width: 640,
                height: 400,
                crt: self.crt_enabled,
                composite: false,
                composite_phase: 0,
            }
        } else {
            let (width, height) = self.machine.display_dimensions();
            RenderInstructions {
                framebuffer: self.machine.display_framebuffer(),
                width,
                height,
                crt: self.crt_enabled,
                composite: self.config.target == Target::Pc60,
                composite_phase: self.composite_phase,
            }
        };

        self.graphics_engine
            .render_frame(window, Some(&render_instructions))
            .context("Graphics engine failed to render frame")?;

        Ok(())
    }
}

const fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        RemovableMediaAction, RemovableMediaRollback, RemovableMediaSelection,
        RemovableMediaTransaction, apply_media_and_state_transactionally,
        image_selector::ImageEntry, plan_removable_media_actions,
    };

    fn media_binding(
        identifier: &str,
        kind: save_state::MediaKind,
        index: u32,
        path: Option<&str>,
    ) -> save_state::MediaBinding {
        save_state::MediaBinding {
            identifier: save_state::MediaBindingId::new(identifier).unwrap(),
            slot: save_state::MediaSlot::new(kind, index),
            source_path: path
                .map(|path| save_state::MediaSourcePath::from_path(std::path::Path::new(path))),
            media_type: match kind {
                save_state::MediaKind::Floppy => "floppy",
                save_state::MediaKind::HardDisk => "hard-disk",
                save_state::MediaKind::CdRom => "cdrom",
                save_state::MediaKind::Cassette => "cassette",
            }
            .to_owned(),
            identity: save_state::ResourceIdentity::from_bytes(identifier.as_bytes()),
            geometry: None,
            write_protected: kind == save_state::MediaKind::CdRom,
            backend_generation: None,
        }
    }

    fn media_mismatch(
        expected: Vec<save_state::MediaBinding>,
        active: Vec<save_state::MediaBinding>,
    ) -> save_state::MediaMismatch {
        save_state::MediaManifest::new(expected)
            .unwrap()
            .compare_current(&save_state::MediaManifest::new(active).unwrap())
            .unwrap()
            .unwrap()
    }

    #[test]
    fn removable_media_plan_selects_and_ejects_all_differing_drives() {
        let mismatch = media_mismatch(
            vec![
                media_binding(
                    "cassette-0",
                    save_state::MediaKind::Cassette,
                    0,
                    Some("media/program.tap"),
                ),
                media_binding(
                    "floppy-0",
                    save_state::MediaKind::Floppy,
                    0,
                    Some("./media/first.d88"),
                ),
                media_binding(
                    "floppy-1",
                    save_state::MediaKind::Floppy,
                    1,
                    Some("media/second.d88"),
                ),
            ],
            vec![
                media_binding(
                    "cdrom-0",
                    save_state::MediaKind::CdRom,
                    0,
                    Some("media/disc.cue"),
                ),
                media_binding(
                    "floppy-0",
                    save_state::MediaKind::Floppy,
                    0,
                    Some("media/other.d88"),
                ),
            ],
        );
        let fdd1_entries = vec![
            ImageEntry::new(PathBuf::from("media/other.d88")),
            ImageEntry::new(PathBuf::from("games/../media/first.d88")),
        ];
        let fdd2_entries = vec![ImageEntry::new(PathBuf::from("media/second.d88"))];
        let cdrom_entries = vec![ImageEntry::new(PathBuf::from("media/disc.cue"))];

        let actions = plan_removable_media_actions(
            &mismatch,
            &fdd1_entries,
            &fdd2_entries,
            &cdrom_entries,
            Some(std::path::Path::new("games/../media/program.tap")),
        )
        .unwrap();

        assert_eq!(
            actions,
            vec![
                RemovableMediaAction::SelectCassette,
                RemovableMediaAction::EjectCdRom,
                RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                RemovableMediaAction::SelectFloppy { drive: 1, index: 0 },
            ]
        );
    }

    #[test]
    fn removable_media_plan_rejects_unconfigured_and_fixed_media() {
        let unconfigured = media_mismatch(
            vec![media_binding(
                "floppy-0",
                save_state::MediaKind::Floppy,
                0,
                Some("missing.d88"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&unconfigured, &[], &[], &[], None)
                .unwrap_err()
                .contains("not configured")
        );

        let hard_disk = media_mismatch(
            vec![media_binding(
                "ide-0",
                save_state::MediaKind::HardDisk,
                0,
                Some("disk.hdd"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&hard_disk, &[], &[], &[], None)
                .unwrap_err()
                .contains("hard disk")
        );

        let cassette = media_mismatch(
            vec![media_binding(
                "cassette-0",
                save_state::MediaKind::Cassette,
                0,
                Some("program.tap"),
            )],
            Vec::new(),
        );
        assert!(
            plan_removable_media_actions(&cassette, &[], &[], &[], None)
                .unwrap_err()
                .contains("not configured")
        );
    }

    #[test]
    fn failed_media_switch_restores_the_previous_selection() {
        #[derive(Default)]
        struct FakeMediaTarget {
            applied: Vec<RemovableMediaAction>,
            restored: Option<RemovableMediaRollback>,
            state: u32,
        }

        let previous = RemovableMediaRollback {
            selection: RemovableMediaSelection {
                floppy: [Some(3), Some(4)],
                cdrom: Some(2),
                cassette: Some(true),
            },
            slots: vec![
                save_state::MediaSlot::new(save_state::MediaKind::Floppy, 0),
                save_state::MediaSlot::new(save_state::MediaKind::Floppy, 1),
            ],
        };
        let actions = vec![
            RemovableMediaAction::SelectFloppy { drive: 0, index: 0 },
            RemovableMediaAction::SelectFloppy { drive: 1, index: 1 },
        ];
        let mut target = FakeMediaTarget {
            state: 99,
            ..FakeMediaTarget::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions,
                previous_media: previous.clone(),
                previous_state: 7,
            },
            |target, action| {
                target.applied.push(action);
                target.state += 1;
                if target.applied.len() == 2 {
                    return Err("second switch failed".into());
                }
                Ok(())
            },
            |_| Ok(()),
            |target, selection| {
                target.restored = Some(selection.clone());
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Err("second switch failed".into()));
        assert_eq!(target.restored, Some(previous));
        assert_eq!(target.state, 7);
    }

    #[test]
    fn successful_media_switch_applies_snapshot_without_rollback() {
        #[derive(Default)]
        struct FakeTarget {
            media: Vec<RemovableMediaAction>,
            media_restored: bool,
            state: u32,
        }

        let mut target = FakeTarget {
            state: 7,
            ..Default::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions: vec![
                    RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                    RemovableMediaAction::SelectCdRom { index: 2 },
                ],
                previous_media: RemovableMediaRollback {
                    selection: RemovableMediaSelection {
                        floppy: [Some(0), None],
                        cdrom: Some(0),
                        cassette: None,
                    },
                    slots: vec![
                        save_state::MediaSlot::new(save_state::MediaKind::Floppy, 0),
                        save_state::MediaSlot::new(save_state::MediaKind::CdRom, 0),
                    ],
                },
                previous_state: 7,
            },
            |target, action| {
                target.media.push(action);
                Ok(())
            },
            |target| {
                target.state = 99;
                Ok(())
            },
            |target, _| {
                target.media_restored = true;
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Ok(()));
        assert_eq!(
            target.media,
            vec![
                RemovableMediaAction::SelectFloppy { drive: 0, index: 1 },
                RemovableMediaAction::SelectCdRom { index: 2 },
            ],
        );
        assert_eq!(target.state, 99);
        assert!(!target.media_restored);
    }

    #[test]
    fn failed_state_restore_rolls_back_media_and_machine_state() {
        #[derive(Default)]
        struct FakeTarget {
            media_restored: bool,
            state: u32,
        }

        let previous = RemovableMediaRollback {
            selection: RemovableMediaSelection {
                floppy: [Some(1), None],
                cdrom: None,
                cassette: None,
            },
            slots: vec![save_state::MediaSlot::new(save_state::MediaKind::Floppy, 0)],
        };
        let mut target = FakeTarget {
            state: 42,
            ..Default::default()
        };
        let result = apply_media_and_state_transactionally(
            &mut target,
            RemovableMediaTransaction {
                actions: vec![RemovableMediaAction::SelectFloppy { drive: 0, index: 0 }],
                previous_media: previous,
                previous_state: 7,
            },
            |target, _| {
                target.state = 43;
                Ok(())
            },
            |target| {
                target.state = 100;
                Err("snapshot payload is invalid".to_owned())
            },
            |target, _| {
                target.media_restored = true;
                Ok(())
            },
            |target, state| {
                target.state = *state;
                Ok(())
            },
        );

        assert_eq!(result, Err("snapshot payload is invalid".to_owned()));
        assert!(target.media_restored);
        assert_eq!(target.state, 7);
    }
}
