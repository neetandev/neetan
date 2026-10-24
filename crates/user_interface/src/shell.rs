//! The shell: the SDL3 context, the window, the engines and the GUI host of
//! the process, the event loop and the start and the end of a session.

mod setup;

use std::time::{Duration, Instant};

use audio_engine::AudioEngine;
use common::{Context, Error, StringError, error, info, warn};
use sdl3::{
    Sdl,
    event::{DisplayEvent, Event, EventPump, WindowEvent},
    keyboard::Scancode,
    video::VideoSubsystem,
};
use sdl3_backend::DisplayAspectMode;

use self::setup::{
    SdlSubsystems, build_window, initialize_sdl3, print_system_info, select_graphics_backend,
};
use crate::{
    GAME_NAME, Result,
    config::{Backend, EmulatorConfig, GraphicboardType, WindowMode},
    gui_host::GuiHost,
    host::{
        Host, HostParts, WindowState, aspect_ratio_for_mode, graphics_display_aspect_mode,
        graphics_scaling, initial_window_size,
    },
    input::KeyOverrides,
    on_off,
    session::{Session, SessionExit, catch_panic},
};

/// The interval of the CPU usage in the window title.
const WINDOW_TITLE_INTERVAL: Duration = Duration::from_secs(5);

/// The settings of the window and the engines of a new shell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShellOptions {
    /// The requested graphics backend.
    backend: Backend,
    /// The playback volume (0.0 to 1.0).
    audio_volume: f32,
    /// The size of the new window in screen coordinates.
    window_size: (u32, u32),
    /// Whether the window starts in fullscreen mode.
    fullscreen: bool,
    /// The display aspect mode of the graphics engine.
    display_aspect_mode: DisplayAspectMode,
    /// Whether the graphics engine needs the large native surface.
    large_native_target: bool,
}

impl ShellOptions {
    /// The shell settings for the game of `config`.
    pub(crate) fn for_config(config: &EmulatorConfig) -> Self {
        Self {
            backend: config.backend,
            audio_volume: config.audio_volume,
            window_size: initial_window_size(config.aspect_mode),
            fullscreen: config.window_mode == WindowMode::Fullscreen,
            display_aspect_mode: graphics_display_aspect_mode(config.aspect_mode),
            large_native_target: config.graphicboard != GraphicboardType::None
                || config.target.wants_large_native_surface(),
        }
    }
}

/// The process wide part of the application. It owns the session.
///
/// The field order is the drop order: the session flushes its media before
/// the engines, the window and SDL3 go.
pub(crate) struct Shell {
    /// The running game.
    session: Option<Session>,
    /// The window, the engines, the gamepad and the GUI host.
    host: Host,
    /// The window state before the session started.
    window_before_session: Option<WindowState>,
    /// The SDL3 video subsystem. Kept alive while the window exists.
    _video_subsystem: VideoSubsystem,
    /// The SDL3 context.
    sdl_context: Sdl,
    /// Whether the application quits.
    should_quit: bool,
    /// Accumulated emulation busy time in the current measurement window.
    busy_duration: Duration,
    /// When the window title was last updated with CPU usage.
    window_title_last_update: Instant,
}

impl Shell {
    /// Initializes SDL3 and creates the hidden window, the graphics engine,
    /// the paused audio engine and the GUI host.
    pub(crate) fn new(options: ShellOptions) -> Result<Self> {
        let SdlSubsystems {
            context: sdl_context,
            audio: audio_subsystem,
            video: video_subsystem,
            gamepad: gamepad_subsystem,
        } = initialize_sdl3()?;

        print_system_info();

        let (width, height) = options.window_size;
        let mut window = build_window(&video_subsystem, width, height, options.fullscreen)?;

        let (graphics_engine, backend) = select_graphics_backend(
            options.display_aspect_mode,
            options.backend,
            options.large_native_target,
            &mut window,
        )?;

        let audio_engine = AudioEngine::new(audio_subsystem, options.audio_volume)
            .context("Failed to initialize audio")?;

        let host = Host::new(HostParts {
            window,
            graphics_engine,
            backend,
            audio_engine,
            gamepad_subsystem,
            fullscreen: options.fullscreen,
        });

        Ok(Self {
            session: None,
            window_before_session: None,
            host,
            _video_subsystem: video_subsystem,
            sdl_context,
            should_quit: false,
            busy_duration: Duration::ZERO,
            window_title_last_update: Instant::now(),
        })
    }

    /// Creates the session of `config` and applies its window and display
    /// settings. A panic while the machine starts becomes an error.
    pub(crate) fn start_session(
        &mut self,
        config: EmulatorConfig,
        key_overrides: KeyOverrides,
    ) -> Result<()> {
        let backend = self.host.backend;
        let session = match catch_panic(|| Session::new(config, key_overrides, backend)) {
            Ok(result) => result?,
            Err(message) => {
                return Err(Error::from(StringError(format!(
                    "the machine panicked while starting: {message}"
                ))));
            }
        };
        self.begin_session(session);
        Ok(())
    }

    /// Records the window state, applies the window mode, the aspect ratio
    /// lock and the display settings of `session`, resumes the audio and keeps
    /// the session.
    fn begin_session(&mut self, session: Session) {
        let host = &mut self.host;
        self.window_before_session = Some(host.window_state());
        let fullscreen = session.window_mode() == WindowMode::Fullscreen;
        if fullscreen != host.fullscreen {
            host.set_fullscreen(fullscreen);
        }
        if !fullscreen {
            let aspect_ratio = aspect_ratio_for_mode(session.aspect_mode());
            if let Err(error) = host.window.set_aspect_ratio(aspect_ratio) {
                warn!("Failed to lock window aspect ratio to {aspect_ratio}: {error}");
            }
        }
        host.graphics_engine
            .set_scaling(graphics_scaling(session.scaling()));
        match host.backend {
            Backend::Legacy => {
                if session.crt_requested() {
                    warn!("CRT filter is not supported by the legacy SDL backend");
                }
            }
            Backend::Modern => info!("CRT effect set to {}", on_off(session.crt_enabled())),
        }
        info!("Scaling set to {}", session.scaling());
        host.audio_engine.resume();
        self.session = Some(session);
    }

    /// Ends the session: flushes and drops it, releases the mouse capture,
    /// clears the aspect ratio lock, restores the window size and fullscreen
    /// state of the time before the session, stops the audio, resets the GUI host and
    /// restores the window title. Returns the reason the session ended, or
    /// `None` without a session or when the session ended without a reason.
    pub(crate) fn end_session(&mut self) -> Option<SessionExit> {
        let session = self.session.take()?;
        let exit = session.exit().cloned();
        if let Err(message) = catch_panic(move || drop(session)) {
            error!("Closing the session panicked: {message}");
        }

        let host = &mut self.host;
        if let Err(error) = host.window.set_relative_mouse_mode(false) {
            warn!("Failed to release the relative mouse mode: {error}");
        }
        if let Err(error) = host.window.clear_aspect_ratio() {
            warn!("Failed to clear the window aspect ratio lock: {error}");
        }
        if let Some(state) = self.window_before_session.take() {
            host.restore_window_state(state);
        }
        host.audio_engine.stop();
        if let Some(gui) = host.gui.as_mut() {
            gui.stop_text_input(Some(&host.window));
            gui.reset();
        }
        host.window.set_title(GAME_NAME);
        self.busy_duration = Duration::ZERO;
        self.window_title_last_update = Instant::now();
        exit
    }

    /// Shows the window and runs the event loop until the application quits
    /// or the session ends. Ends the session. Returns an error when the
    /// session ended with a panic.
    pub(crate) fn run(&mut self) -> Result<()> {
        self.host.window.show();

        let mut event_pump = self
            .sdl_context
            .event_pump()
            .context("Failed to get the SDL3 event pump")?;

        self.run_session_loop(&mut event_pump);

        match self.end_session() {
            Some(SessionExit::Panic { message }) => Err(Error::from(StringError(format!(
                "the session ended after a panic of the emulator: {message}"
            )))),
            Some(SessionExit::Quit | SessionExit::GuestShutdown) | None => Ok(()),
        }
    }

    /// Runs the session loop. The audio engine paces the loop. The loop ends
    /// when the application quits, the session ends or no session exists.
    fn run_session_loop(&mut self, event_pump: &mut EventPump) {
        'running: loop {
            for event in event_pump.poll_iter() {
                self.handle_event(&event);
                if self.should_quit || self.session_ending() {
                    break 'running;
                }
            }

            let Some(session) = self.session.as_mut() else {
                break;
            };
            let host = &mut self.host;

            let busy_start = Instant::now();
            session.guarded(|session| session.run_frame(host));
            self.busy_duration += busy_start.elapsed();

            session.guarded(|session| session.update_dialog(host));
            if session.exit().is_some() {
                break;
            }

            render_session_frame(session, host);
            self.update_window_title();
        }
    }

    /// Whether the session has a reason to end.
    fn session_ending(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.exit().is_some())
    }

    /// Shows the CPU usage of the emulation in the window title.
    fn update_window_title(&mut self) {
        let elapsed = self.window_title_last_update.elapsed();
        if elapsed < WINDOW_TITLE_INTERVAL {
            return;
        }
        let busy_percent = (self.busy_duration.as_secs_f64() / elapsed.as_secs_f64() * 100.0)
            .round()
            .min(100.0) as u32;
        self.host
            .window
            .set_title(&format!("{GAME_NAME} ({busy_percent}% CPU)"));
        self.busy_duration = Duration::ZERO;
        self.window_title_last_update = Instant::now();
    }

    /// Handles the window, focus and gamepad connection events and passes
    /// every other event to the session.
    fn handle_event(&mut self, event: &Event) {
        let host = &mut self.host;
        match event {
            Event::Quit => {
                self.should_quit = true;
            }
            Event::Window {
                win_event: WindowEvent::PixelSizeChanged(..),
                ..
            } => {
                host.update_gui_window();
            }
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => {
                if let Some(session) = self.session.as_mut() {
                    session.guarded(|session| session.focus_lost(host));
                    host.audio_engine.pause();
                }
                host.right_control_held = false;
                if let Some(gui) = host.gui.as_mut() {
                    gui.gamepad.clear();
                }
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => {
                if self
                    .session
                    .as_ref()
                    .is_some_and(|session| !session.media_dialog_open())
                {
                    host.audio_engine.resume();
                }
            }
            Event::Window {
                win_event: WindowEvent::DisplayScaleChanged,
                ..
            }
            | Event::Display {
                display_event: DisplayEvent::ContentScaleChanged,
                ..
            } => {
                let scale = host.window.display_scale();
                if scale != host.scale_factor {
                    host.scale_factor = scale;
                    info!("Scale factor changed to: {scale}");
                }
                host.update_gui_window();
            }
            Event::GamepadAdded { which } => {
                host.open_gamepad(*which);
            }
            Event::GamepadRemoved { which } => {
                if host.close_gamepad(*which)
                    && let Some(session) = self.session.as_mut()
                {
                    session.guarded(|session| session.gamepad_disconnected(host));
                }
            }
            _ => {
                match event {
                    Event::KeyDown {
                        scancode, keymod, ..
                    } if *scancode == Some(Scancode::RCtrl) || keymod.rctrl() => {
                        host.right_control_held = true;
                    }
                    Event::KeyUp {
                        scancode: Some(Scancode::RCtrl),
                        ..
                    } => {
                        host.right_control_held = false;
                    }
                    _ => {}
                }
                if let Some(session) = self.session.as_mut() {
                    session.guarded(|session| session.handle_event(event, host));
                }
            }
        }
    }
}

/// Renders the frame of `session` with the GUI overlay of an open media
/// dialog. A panic while reading the frame ends the session.
fn render_session_frame(session: &mut Session, host: &mut Host) {
    let overlay_shown = session.media_dialog_open();
    let result = match session.render_instructions() {
        Ok(instructions) => {
            let overlay = if overlay_shown {
                host.gui.as_ref().map(GuiHost::overlay)
            } else {
                None
            };
            host.graphics_engine
                .render_frame(&host.window, Some(&instructions), overlay)
                .context("Graphics engine failed to render frame")
        }
        Err(message) => {
            session.record_panic(message);
            return;
        }
    };
    if let Err(error) = result {
        error!("Failed to render next frame: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use sdl3_backend::DisplayAspectMode;

    use super::ShellOptions;
    use crate::config::{AspectMode, Backend, EmulatorConfig, GraphicboardType, WindowMode};

    #[test]
    fn options_follow_the_window_settings_of_the_game() {
        let config = EmulatorConfig {
            aspect_mode: AspectMode::Aspect1By1,
            window_mode: WindowMode::Fullscreen,
            backend: Backend::Legacy,
            audio_volume: 0.5,
            ..EmulatorConfig::default()
        };
        let options = ShellOptions::for_config(&config);
        assert_eq!(options.window_size, (1280, 800));
        assert!(options.fullscreen);
        assert_eq!(options.backend, Backend::Legacy);
        assert_eq!(options.audio_volume, 0.5);
        assert_eq!(options.display_aspect_mode, DisplayAspectMode::Aspect1By1);
    }

    #[test]
    fn a_graphic_board_needs_the_large_native_target() {
        let config = EmulatorConfig::default();
        assert!(!config.target.wants_large_native_surface());
        assert!(!ShellOptions::for_config(&config).large_native_target);

        let with_board = EmulatorConfig {
            graphicboard: GraphicboardType::Ga1280a,
            ..EmulatorConfig::default()
        };
        assert!(ShellOptions::for_config(&with_board).large_native_target);
    }
}
