//! Neetan, a PC-98 emulator.

#![deny(unsafe_code)]

use std::{
    fs::File,
    time::{Duration, Instant},
};

use audio_engine::AudioEngine;
use common::{
    BUILTIN_FONT_ROM, Context, CpuMode, JoystickState, Machine, MachineModel, StringError, bail,
    ensure, error, info, warn,
};
use device::disk::{HddGeometry, load_hdd_image};
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
    config::{AspectMode, Backend, EmulatorConfig, ForceGdcClock, ScalingMode, Target, WindowMode},
    errors::Error,
    image_selector::{ImageEntry, ImageSelector, MediaType},
    keyboard::{JoystickKey, KeyMap, KeyboardForwardingState},
};

pub mod config;
pub mod convert;
pub mod copy;
pub mod create;
mod errors;
mod image_selector;
mod keyboard;

#[cfg(feature = "tracing")]
mod tracing;

#[cfg(feature = "tracing")]
type Tracer = crate::tracing::Tracing;
#[cfg(not(feature = "tracing"))]
type Tracer = machine::NoTracing;

pub const COMPANY_NAME: &str = "neetan";
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

pub type Result<T> = std::result::Result<T, Error>;

pub fn run(config: EmulatorConfig) -> Result<()> {
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
            window.set_title(&format!("neetan ({busy_percent}% CPU)"));
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
    let ga_enabled = config.graphicboard != config::GraphicboardType::None;
    match config.backend {
        Backend::Legacy => {
            info!("Using legacy backend");
            let mut engine = LegacySdlBackend::new(aspect_mode, ga_enabled);
            engine
                .on_resume(window, true)
                .context("Failed to initialize legacy SDL backend")?;
            Ok((Box::new(engine), Backend::Legacy))
        }
        Backend::Modern => {
            let modern_result = ModernSdlGpuBackend::new(aspect_mode, ga_enabled)
                .and_then(|mut engine| engine.on_resume(window, true).map(|()| engine));

            match modern_result {
                Ok(engine) => {
                    info!("Using modern backend");
                    Ok((Box::new(engine), Backend::Modern))
                }
                Err(error) => {
                    warn!("Modern backend unavailable, falling back to legacy backend: {error:#}");
                    let mut legacy = LegacySdlBackend::new(aspect_mode, ga_enabled);
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
    /// Whether relative mouse mode is active (Right Ctrl toggles).
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
    /// Host-to-PC-98 key mapping.
    key_map: KeyMap,
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
    /// Composite subcarrier phase select (0..3), cycled with Alt+F3. Swaps the
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

impl Drop for Application {
    fn drop(&mut self) {
        self.machine.flush_printer();
        self.machine.flush_floppies();
        self.machine.flush_hdds();
    }
}

impl Application {
    pub(crate) fn new(
        config: EmulatorConfig,
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

        let mut machine = initialize_machine(&config, audio_engine::SAMPLE_RATE as u32)?;
        let selector_font_rom = selector_font_rom_data(&config, machine.as_ref());

        if config.enable_extractor {
            machine.install_text_extractor(Box::new(text_extractor::ClipboardExtractor::new()));
            info!("Text extractor enabled (clipboard sink)");
        }

        let key_map = config.key_map;

        let mut fdd1_index = None;
        if let Some(entry) = fdd1_entries.first() {
            match machine.insert_floppy(0, &entry.path) {
                Ok(desc) => {
                    info!("Inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(e) => return Err(Error::from(StringError(e))),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = fdd2_entries.first() {
            match machine.insert_floppy(1, &entry.path) {
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
            key_map,
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
                        self.handle_selector_key(*scancode, keymod.alt_gui());
                    }
                } else {
                    self.keyboard_forwarding_state.handle_key_down(
                        *scancode,
                        keymod.gui(),
                        keymod.shift(),
                        keymod.ctrl(),
                        *repeat,
                        &self.key_map,
                    );
                    self.keyboard_forwarding_state
                        .apply_pending_actions(self.machine.as_mut());

                    // Keyboard joystick fallback (only used when no gamepad is
                    // connected): drive the joystick directions/triggers.
                    if let Some(key) = JoystickKey::from_scancode(*scancode) {
                        key.apply(&mut self.keyboard_joystick, true);
                        self.update_joystick();
                    }

                    // Right Ctrl toggles mouse capture.
                    if !repeat
                        && *scancode == Some(Scancode::RCtrl)
                        && let Some(w) = window
                    {
                        self.toggle_mouse_capture(w);
                    }

                    if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::Escape) {
                        self.should_quit = true;
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::Return) {
                        if let Some(w) = window {
                            if let Err(error) = w.set_fullscreen(!self.fullscreen) {
                                warn!("Failed to toggle fullscreen: {error}");
                            } else {
                                self.fullscreen = !self.fullscreen;
                            }
                        }
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F1) {
                        self.toggle_crt();
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F2) {
                        self.toggle_scaling();
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F3) {
                        self.cycle_composite_phase();
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F9) {
                        self.open_or_toggle_selector(MediaType::Floppy(0));
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F10) {
                        self.open_or_toggle_selector(MediaType::Floppy(1));
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::F11) {
                        self.open_or_toggle_selector(MediaType::CdRom);
                    } else if !repeat && keymod.alt_gui() && *scancode == Some(Scancode::R) {
                        self.hard_reset();
                    } else if keymod.alt_gui() && *scancode == Some(Scancode::F) {
                        self.set_fast_forward(true);
                    }
                }
            }
            Event::KeyUp {
                scancode, repeat, ..
            } => {
                if self.fast_forward
                    && matches!(
                        scancode,
                        Some(Scancode::F) | Some(Scancode::LAlt) | Some(Scancode::RAlt)
                    )
                {
                    self.set_fast_forward(false);
                }
                if self.image_selector.is_none() {
                    if let Some(code) = self
                        .keyboard_forwarding_state
                        .handle_key_up(*scancode, *repeat)
                    {
                        self.machine.push_keyboard_scancode(code);
                    }
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
            GamepadButton::South => self.gamepad_buttons.trigger1 = pressed,
            GamepadButton::East => self.gamepad_buttons.trigger2 = pressed,
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
        let state = if self.gamepad.is_some() {
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

    fn select_floppy(&mut self, drive: usize, index: usize) {
        let entries = match drive {
            0 => &self.fdd1_entries,
            1 => &self.fdd2_entries,
            _ => return,
        };

        if index >= entries.len() {
            return;
        }

        self.machine.eject_floppy(drive);

        let path = &entries[index].path;
        match self.machine.insert_floppy(drive, path) {
            Ok(desc) => info!("Selected FDD{}: {desc} from {}", drive + 1, path.display()),
            Err(error) => error!("Failed to select FDD{}: {error}", drive + 1),
        }

        match drive {
            0 => self.fdd1_index = Some(index),
            1 => self.fdd2_index = Some(index),
            _ => {}
        }
    }

    fn eject_cdrom(&mut self) {
        self.machine.eject_cdrom();
        self.cdrom_index = None;
        info!("Ejected CD-ROM");
    }

    fn select_cdrom(&mut self, index: usize) {
        if index >= self.cdrom_entries.len() {
            return;
        }

        self.machine.eject_cdrom();

        let path = &self.cdrom_entries[index].path;
        match self.machine.insert_cdrom(path) {
            Ok(desc) => info!("Selected CD-ROM: {desc} from {}", path.display()),
            Err(error) => error!("Failed to select CD-ROM: {error}"),
        }

        self.cdrom_index = Some(index);
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

    fn handle_selector_key(&mut self, scancode: Option<Scancode>, alt_gui_held: bool) {
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
                                self.select_floppy(*drive, cursor - 1);
                            }
                        }
                        MediaType::CdRom => {
                            if cursor == 0 {
                                self.eject_cdrom();
                            } else {
                                self.select_cdrom(cursor - 1);
                            }
                        }
                    }
                }
                self.close_selector();
            }
            Scancode::Escape => {
                self.close_selector();
            }
            Scancode::F9 if alt_gui_held => {
                self.open_or_toggle_selector(MediaType::Floppy(0));
            }
            Scancode::F10 if alt_gui_held => {
                self.open_or_toggle_selector(MediaType::Floppy(1));
            }
            Scancode::F11 if alt_gui_held => {
                self.open_or_toggle_selector(MediaType::CdRom);
            }
            _ => {}
        }
    }

    fn hard_reset(&mut self) {
        self.machine.flush_printer();
        self.machine.flush_floppies();
        self.machine.flush_hdds();

        let mut machine = match initialize_machine(&self.config, audio_engine::SAMPLE_RATE as u32) {
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
            match machine.insert_floppy(0, &entry.path) {
                Ok(desc) => {
                    info!("Re-inserted FDD1: {desc} from {}", entry.path.display());
                    fdd1_index = Some(0);
                }
                Err(error) => warn!("Hard reset failed to re-insert FDD1: {error}"),
            }
        }
        let mut fdd2_index = None;
        if let Some(entry) = self.fdd2_entries.first() {
            match machine.insert_floppy(1, &entry.path) {
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

/// Returns the current host local time as a 6-byte BCD buffer for the µPD4990A RTC.
///
/// Format: `[year, month<<4|day_of_week, day, hour, minute, second]`.
fn host_local_time_bcd() -> [u8; 6] {
    fn to_bcd(value: u8) -> u8 {
        ((value / 10) << 4) | (value % 10)
    }

    let Ok(dt) = sdl3::time::local_date_time() else {
        return [0; 6];
    };

    let year = to_bcd((dt.year % 100) as u8);
    let month_dow = ((dt.month as u8) << 4) | (dt.day_of_week as u8);
    let day = to_bcd(dt.day as u8);
    let hour = to_bcd(dt.hour as u8);
    let minute = to_bcd(dt.minute as u8);
    let second = to_bcd(dt.second as u8);

    [year, month_dow, day, hour, minute, second]
}

pub fn initialize_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    match config.target {
        Target::Pc98 => initialize_pc98_machine(config, sample_rate),
        Target::Pc88 => initialize_pc88_machine(config, sample_rate),
        Target::Pc88Va => initialize_pc88va_machine(config, sample_rate),
        Target::Pc60 => initialize_pc60_machine(config, sample_rate),
    }
}

fn selector_font_rom_data(config: &EmulatorConfig, machine: &dyn Machine) -> Vec<u8> {
    if config.target == Target::Pc98 {
        return machine.font_rom_data().to_vec();
    }

    expand_selector_font_rom(BUILTIN_FONT_ROM)
}

fn expand_selector_font_rom(raw_font_rom: &[u8]) -> Vec<u8> {
    let mut bus: machine::Pc9801Bus<machine::NoTracing> = machine::Pc9801Bus::new(
        MachineModel::PC9801VM,
        CpuMode::High,
        audio_engine::SAMPLE_RATE as u32,
    );
    bus.load_font_rom(raw_font_rom);
    bus.font_rom_data().to_vec()
}

fn initialize_pc88_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    let model = machine88::Pc8801Model::PC8801MC;
    info!("Selected machine model {model}");

    let rom_dir = config.pc88_roms.as_ref().ok_or_else(|| {
        StringError("PC-8801MC requires a ROM directory (--pc88-roms <DIR>)".into())
    })?;

    let roms = machine88::load_rom_set(rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load PC-8801MC ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    roms.validate_for_boot_mode(config.pc88_boot_mode)
        .map_err(|error| {
            StringError(format!(
                "PC-8801MC boot mode '{}' requires a ROM missing from {}: {error}",
                config.pc88_boot_mode,
                rom_dir.display()
            ))
        })?;

    let clock_select = match config.cpu_mode {
        common::CpuMode::Low => machine88::ClockSelect::FourMhz,
        common::CpuMode::High => machine88::ClockSelect::EightMhz,
    };

    let mut bus: machine88::Pc8801Bus = machine88::Pc8801Bus::new(model, clock_select, sample_rate);
    bus.set_boot_mode(config.pc88_boot_mode);
    bus.set_monitor_timing(config.pc88_monitor);
    bus.set_memory_wait(config.pc88_memory_wait);
    bus.set_eight_mhz_wait(config.pc88_8mhz_wait);
    bus.set_host_local_time_fn(host_local_time_bcd);
    bus.load_roms(&roms);

    info!(
        "PC-8801MC configured: {} clock, boot mode {}, monitor {}, memory wait {}, 8 MHz wait {}",
        match config.cpu_mode {
            CpuMode::Low => "4 MHz",
            CpuMode::High => "8 MHz",
        },
        config.pc88_boot_mode,
        config.pc88_monitor,
        config.pc88_memory_wait,
        config.pc88_8mhz_wait
    );

    if config.hdd1.is_some() || config.hdd2.is_some() {
        warn!("HDD options are ignored for the PC-8801MC target");
    }

    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let sub_cpu = cpu::Z80::new(bus.sub_clock_hz());
    Ok(Box::new(machine88::Pc8801Machine::new(
        main_cpu, sub_cpu, bus,
    )))
}

fn initialize_pc88va_machine(
    config: &EmulatorConfig,
    _sample_rate: u32,
) -> Result<Box<dyn Machine>> {
    let model = config.pc88va_model;
    info!("Selected machine model {model}");

    let rom_dir = config.pc88va_roms.as_ref().ok_or_else(|| {
        StringError("PC-88VA requires a ROM directory (--pc88va-roms <DIR>)".into())
    })?;

    let roms = machine88va::load_rom_set(rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load PC-88VA ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    if config.hdd1.is_some() || config.hdd2.is_some() {
        warn!("HDD options are ignored for the PC-88VA target");
    }
    if !config.cdrom.is_empty() {
        warn!("CD-ROM options are ignored for the PC-88VA target");
    }

    let mut machine = machine88va::Pc88VaMachine::new(model, roms);
    machine.set_host_local_time_fn(host_local_time_bcd);
    Ok(Box::new(machine))
}

fn initialize_pc60_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    let model = config.pc60_model;
    info!("Selected machine model {model}");

    let rom_dir = config.pc60_roms.as_ref().ok_or_else(|| {
        StringError(format!(
            "{model} requires a ROM directory (--pc60-roms <DIR>)"
        ))
    })?;

    let roms = machine60::load_rom_set(model, rom_dir).map_err(|error| {
        StringError(format!(
            "Failed to load {model} ROM set from {}: {error}",
            rom_dir.display()
        ))
    })?;

    let mut bus: machine60::Pc6000Bus<Tracer> = machine60::Pc6000Bus::new(model, sample_rate);
    bus.load_roms(&roms);

    if let Some(cart_path) = config.pc60_cart.as_ref() {
        let image = std::fs::read(cart_path).map_err(|error| {
            StringError(format!(
                "Failed to read PC-6000 cartridge {}: {error}",
                cart_path.display()
            ))
        })?;
        info!(
            "Loaded cartridge {} ({} bytes)",
            cart_path.display(),
            image.len()
        );
        bus.load_cartridge(&image);
    }

    if config.hdd1.is_some() || config.hdd2.is_some() {
        warn!("HDD options are ignored for the PC-6000 target");
    }

    let main_cpu = cpu::Z80::new(bus.cpu_clock_hz());
    let mut machine = machine60::Pc6000Machine::new(main_cpu, bus);

    if let Some(cassette_path) = config.pc60_cass.as_ref() {
        match machine.insert_cassette(cassette_path) {
            Ok(description) => info!("Inserted cassette {description}"),
            Err(error) => {
                return Err(Error::from(StringError(format!(
                    "Failed to insert PC-6000 cassette: {error}"
                ))));
            }
        }
    }

    Ok(Box::new(machine))
}

fn initialize_pc98_machine(config: &EmulatorConfig, sample_rate: u32) -> Result<Box<dyn Machine>> {
    let model = config.machine;

    info!("Selected machine model {model}");

    let mut bus: machine::Pc9801Bus<Tracer> =
        machine::Pc9801Bus::new(model, config.cpu_mode, sample_rate);
    bus.set_host_local_time_fn(host_local_time_bcd);
    bus.set_boot_device(config.boot_device);

    // EMS / XMS configuration gated by machine capability
    if model.ems_compatible() {
        bus.set_ems_enabled(config.ems);
    }
    if model.xms_compatible() {
        bus.set_xms_enabled(config.xms);
        if model.xms_32_compatible() {
            bus.set_xms_32_enabled(config.xms);
        }
    }

    // GDC clock rate configuration logic
    match (model.has_pegc(), model.has_egc(), config.force_gdc_clock) {
        // PEGC machines (PC-9821): default to 5 MHz
        (true, _, None) => {
            bus.set_gdc_clock_5mhz();
        }
        (true, _, Some(ForceGdcClock::Force5)) => {
            bus.set_gdc_clock_5mhz();
            info!("GDC clock forced to 5 MHz (400-line graphics mode)");
        }
        (true, _, Some(ForceGdcClock::Force2_5)) => {
            info!("GDC clock forced to 2.5 MHz (200-line compatibility mode)");
        }
        // EGC-only machines (PC-9801VX/RA): default to 2.5 MHz
        (false, true, Some(ForceGdcClock::Force5)) => {
            bus.set_gdc_clock_5mhz();
            info!("GDC clock forced to 5 MHz (400-line graphics mode)");
        }
        (false, true, Some(ForceGdcClock::Force2_5)) => {
            info!("GDC clock forced to 2.5 MHz (200-line compatibility mode)");
        }
        (false, true, None) => {}
        // Non-EGC machines (PC-9801VM): no 5 MHz support
        (false, false, Some(ForceGdcClock::Force5)) => {
            warn!("{model} does not support 5 MHz GDC clock, ignoring --force-gdc-clock 5");
        }
        (false, false, Some(ForceGdcClock::Force2_5)) | (false, false, None) => {}
    }

    if config.bios && config.debug_bios.is_some() {
        bail!("--bios and --debug-bios cannot be used together");
    }

    let loaded_roms = match config.pc98_roms {
        Some(ref rom_dir) => Some(machine::load_rom_set(model, rom_dir).map_err(|error| {
            StringError(format!(
                "Failed to load PC-98 ROM set from {}: {error}",
                rom_dir.display()
            ))
        })?),
        None => None,
    };

    if let Some(ref debug_path) = config.debug_bios {
        let bios_rom = std::fs::read(debug_path).with_context(|| {
            format!(
                "Failed to read debug BIOS ROM from {}",
                debug_path.display()
            )
        })?;
        ensure!(
            model.is_valid_bios_rom_size(bios_rom.len()),
            "Debug BIOS ROM is {} bytes, which is not a valid size for {}: {}",
            bios_rom.len(),
            model,
            debug_path.display(),
        );
        info!(
            "Loaded debug BIOS ROM ({} bytes) from {}",
            bios_rom.len(),
            debug_path.display()
        );
        bus.load_bios_rom(&bios_rom);
    } else if config.bios {
        let rom_dir = config
            .pc98_roms
            .as_ref()
            .ok_or_else(|| StringError("--bios requires --pc98-roms <DIR>".into()))?;
        if model.is_pc9821() {
            warn!("No real BIOS ROM available for PC-9821; using HLE BIOS");
        } else if let Some(bios_rom) = loaded_roms.as_ref().and_then(|roms| roms.bios.as_ref()) {
            info!("Loaded BIOS ROM ({} bytes) for {}", bios_rom.len(), model);
            bus.load_bios_rom(bios_rom);
        } else {
            bail!(
                "no BIOS ROM for {} found in {} (accepted digests: {})",
                model,
                rom_dir.display(),
                machine::accepted_bios_digests(model).join(", "),
            );
        }
    } else {
        info!("No BIOS ROM selected - running in HLE BIOS mode");
    }

    // Font ROM is best-effort regardless of --bios: use the directory font when
    // present, otherwise the built-in font.
    match loaded_roms.as_ref().and_then(|roms| roms.font.as_ref()) {
        Some(font_rom) => {
            info!("Loaded font ROM ({} bytes) for {}", font_rom.len(), model);
            bus.load_font_rom(font_rom);
        }
        None => {
            info!("Using built-in font ROM ({} bytes)", BUILTIN_FONT_ROM.len());
            bus.load_font_rom(BUILTIN_FONT_ROM);
        }
    }

    match config.graphicboard {
        config::GraphicboardType::None => {}
        config::GraphicboardType::Ga1280a => {
            bus.install_ga1280a();
            info!("Installed I-O DATA GA-1280A graphics accelerator");
        }
    }

    match config.soundboard {
        config::SoundboardType::None => {}
        config::SoundboardType::Sb14 => {
            bus.install_soundboard_14();
            info!("Installed PC-9801-14 sound board (TMS3631 8ch synth)");
        }
        config::SoundboardType::Sb26k => {
            bus.install_soundboard_26k(false);
            info!("Installed PC-9801-26K sound board (YM2203 OPN)");
        }
        config::SoundboardType::Sb86 => {
            bus.install_soundboard_86(None, config.adpcm_ram);
            info!("Installed PC-9801-86 sound board (YM2608 OPNA + PCM86)");
        }
        config::SoundboardType::Sb86And26k => {
            bus.install_soundboard_26k(true);
            info!("Installed PC-9801-26K sound board (YM2203 OPN) at alternate ports");
            bus.install_soundboard_86(None, config.adpcm_ram);
            info!("Installed PC-9801-86 sound board (YM2608 OPNA + PCM86)");
        }
        config::SoundboardType::Sb16 => {
            bus.install_sound_blaster_16();
            info!("Installed Sound Blaster 16 (CT2720, YMF262 OPL3 + CT1741 DSP)");
        }
        config::SoundboardType::Sb16And26k => {
            bus.install_soundboard_26k(false);
            info!("Installed PC-9801-26K sound board (YM2203 OPN)");
            bus.install_sound_blaster_16();
            info!("Installed Sound Blaster 16 (CT2720, YMF262 OPL3 + CT1741 DSP)");
        }
    }

    // The PC-9801-26K sound BIOS ROM (CC000-CFFFF) is loaded only in real-BIOS
    // mode when a 26K board is present; otherwise the built-in stub is kept.
    let has_26k_board = matches!(
        config.soundboard,
        config::SoundboardType::Sb26k
            | config::SoundboardType::Sb86And26k
            | config::SoundboardType::Sb16And26k
    );
    if config.bios && has_26k_board {
        match loaded_roms.as_ref().and_then(|roms| roms.sound.as_ref()) {
            Some(sound_rom) => {
                info!("Loaded PC-9801-26K sound ROM ({} bytes)", sound_rom.len());
                bus.load_sound_rom(Some(sound_rom));
            }
            None => info!("No 26K sound ROM found - using built-in sound BIOS stub"),
        }
    }

    if config.midi == config::MidiDevice::Mt32 {
        if let Some(ref mt32_rom_dir) = config.mt32_roms {
            #[cfg(feature = "mt32")]
            {
                match bus.install_mt32(mt32_rom_dir) {
                    Ok(()) => info!("Loaded MT-32 sound module (munt)"),
                    Err(error) => warn!("MT-32 unavailable: {error}"),
                }
            }
            #[cfg(not(feature = "mt32"))]
            {
                let _ = mt32_rom_dir;
                warn!("MT-32 ROM path specified, but MT-32 support was not compiled in");
            }
        } else {
            warn!("MIDI device set to MT-32, but no MT-32 ROM directory specified (--mt32-roms)");
        }
    }

    if config.midi == config::MidiDevice::Sc55 {
        if let Some(ref sc55_rom_dir) = config.sc55_roms {
            #[cfg(feature = "sc55")]
            {
                match bus.install_sc55(sc55_rom_dir) {
                    Ok(()) => info!("Loaded Nuked-SC55 sound module"),
                    Err(error) => warn!("SC-55 unavailable: {error}"),
                }
            }
            #[cfg(not(feature = "sc55"))]
            {
                let _ = sc55_rom_dir;
                warn!("SC-55 ROM path specified, but SC-55 support was not compiled in");
            }
        } else {
            warn!("MIDI device set to SC-55, but no SC-55 ROM directory specified (--sc55-roms)");
        }
    }

    if let Some(ref printer_path) = config.printer {
        let file = File::options()
            .write(true)
            .open(printer_path)
            .with_context(|| {
                format!("Failed to open printer output: {}", printer_path.display())
            })?;
        info!("Printer attached: {}", printer_path.display());
        bus.attach_printer(file);
    }

    if let Some(ref hdd1_path) = config.hdd1 {
        let data = std::fs::read(hdd1_path)
            .with_context(|| format!("Failed to read HDD1 image from {}", hdd1_path.display()))?;
        let image = load_hdd_image(hdd1_path, &data)
            .with_context(|| format!("Failed to parse HDD1 image from {}", hdd1_path.display()))?;
        validate_hdd_for_machine(model, &image.geometry, "HDD1")?;
        info!(
            "Inserted HDD1: {}C/{}H/{}S ({}) from {}",
            image.geometry.cylinders,
            image.geometry.heads,
            image.geometry.sectors_per_track,
            image.format_name(),
            hdd1_path.display()
        );
        bus.insert_hdd(0, image, Some(hdd1_path.clone()));
    }

    if let Some(ref hdd2_path) = config.hdd2 {
        let data = std::fs::read(hdd2_path)
            .with_context(|| format!("Failed to read HDD2 image from {}", hdd2_path.display()))?;
        let image = load_hdd_image(hdd2_path, &data)
            .with_context(|| format!("Failed to parse HDD2 image from {}", hdd2_path.display()))?;
        validate_hdd_for_machine(model, &image.geometry, "HDD2")?;
        info!(
            "Inserted HDD2: {}C/{}H/{}S ({}) from {}",
            image.geometry.cylinders,
            image.geometry.heads,
            image.geometry.sectors_per_track,
            image.format_name(),
            hdd2_path.display()
        );
        bus.insert_hdd(1, image, Some(hdd2_path.clone()));
    }

    let machine: Box<dyn Machine> = match model.cpu_type() {
        common::CpuType::I8086 => Box::new(machine::Machine::new(cpu::I8086::new(), bus)),
        common::CpuType::V30 => Box::new(machine::Machine::new(cpu::V30::new(), bus)),
        common::CpuType::I286 => Box::new(machine::Machine::new(cpu::I286::new(), bus)),
        common::CpuType::I386 => Box::new(machine::Machine::new(
            cpu::I386::<{ cpu::CPU_MODEL_386 }>::new(),
            bus,
        )),
        common::CpuType::I486DX => Box::new(machine::Machine::new(
            cpu::I386::<{ cpu::CPU_MODEL_486 }>::new(),
            bus,
        )),
    };

    Ok(machine)
}

fn validate_hdd_for_machine(
    model: MachineModel,
    geometry: &HddGeometry,
    label: &str,
) -> Result<()> {
    match model {
        MachineModel::PC9801F
        | MachineModel::PC9801VM
        | MachineModel::PC9801VX
        | MachineModel::PC9801RA => {
            ensure!(
                geometry.sasi_media_type().is_some(),
                "{label} is not compatible with {model} (SASI): \
                 geometry {}C/{}H/{}S with {}-byte sectors \
                 does not match any standard SASI drive type",
                geometry.cylinders,
                geometry.heads,
                geometry.sectors_per_track,
                geometry.sector_size,
            );
        }
        MachineModel::PC9821AS | MachineModel::PC9821AP => {
            ensure!(
                geometry.sector_size == 512 || geometry.sasi_media_type().is_some(),
                "{label} is not compatible with {model} (IDE): \
                 geometry {}C/{}H/{}S with {}-byte sectors \
                 is not a valid IDE or SASI-compatible drive type",
                geometry.cylinders,
                geometry.heads,
                geometry.sectors_per_track,
                geometry.sector_size,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        BUILTIN_FONT_ROM, expand_selector_font_rom,
        image_selector::{ImageEntry, ImageSelector, MediaType},
    };

    #[test]
    fn selector_font_rom_expands_builtin_font_into_cgrom_layout() {
        let font_rom = expand_selector_font_rom(BUILTIN_FONT_ROM);

        assert_eq!(font_rom.len(), 0x83000);
        assert!(
            font_rom[0x80000..0x83000].iter().any(|&byte| byte != 0),
            "expanded ANK font area must not be blank"
        );

        let mut selector = ImageSelector::new(MediaType::Floppy(0), 0, 2, &font_rom);
        let entries = [ImageEntry::new(PathBuf::from("disk.d88"))];
        selector.ensure_render(&entries, Some(0));

        assert!(
            selector
                .framebuffer()
                .chunks_exact(4)
                .any(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0),
            "image selector framebuffer must not be blank"
        );
    }
}
