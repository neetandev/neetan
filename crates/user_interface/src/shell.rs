//! The shell: the SDL3 context, the window, the engines and the GUI host of
//! the process, the game manager, the event loops and the start and the end
//! of a session.

mod manager_loop;
mod session_cycle;
mod setup;

use std::{
    fmt::Display,
    path::PathBuf,
    result,
    time::{Duration, Instant},
};

use audio_engine::AudioEngine;
use common::{Context, Error, StringError, error, info, warn};
use machine_factory::config::{
    DEFAULT_AUDIO_VOLUME, DEFAULT_BACKEND, DEFAULT_GUI_THEME, DEFAULT_WINDOW_MODE,
    settings::{AUDIO_VOLUME_KEY, BACKEND_KEY, GUI_THEME_KEY, WINDOW_MODE_KEY, parse_audio_volume},
};
use sdl3::{
    Sdl,
    event::{DisplayEvent, Event, EventPump, WindowEvent},
    keyboard::{self, Scancode},
    video::VideoSubsystem,
};
use sdl3_backend::DisplayAspectMode;

use self::{
    session_cycle::{AfterSession, after_session},
    setup::{
        SdlSubsystems, build_window, initialize_sdl3, print_system_info, select_graphics_backend,
    },
};
use crate::{
    GAME_NAME, Result,
    config::{
        Backend, EmulatorConfig, GuiLanguage, GuiResources, GuiTheme, HostLayers, WindowMode,
    },
    gui_host::{GuiHost, keys::shift_after, theme_kind},
    host::{
        Host, HostParts, MANAGER_WINDOW_SIZE, MINIMUM_WINDOW_SIZE, SESSION_MINIMUM_WINDOW_SIZE,
        WindowSize, WindowState, fit_window_size, graphics_display_aspect_mode,
        initial_window_size, logical_to_screen, manager_minimum_size,
        screen_units_per_logical_unit, wants_large_native_target,
    },
    manager::{Launch, Manager},
    session::Session,
};

/// The interval of the CPU usage in the window title.
const WINDOW_TITLE_INTERVAL: Duration = Duration::from_secs(5);

/// The settings of the window and the engines of a new shell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShellOptions {
    /// The requested graphics backend.
    backend: Backend,
    /// The playback volume (0.0 to 1.0).
    audio_volume: f32,
    /// The size of the new window.
    window_size: WindowSize,
    /// Whether the window starts in fullscreen mode.
    fullscreen: bool,
    /// The display aspect mode of the graphics engine.
    display_aspect_mode: DisplayAspectMode,
    /// Whether the graphics engine needs the large native surface.
    large_native_target: bool,
    /// The paths searched for the font file of the user interface in search order.
    gui_font_paths: Vec<PathBuf>,
    /// The theme of the user interface.
    gui_theme: GuiTheme,
    /// The language of the user interface.
    gui_language: GuiLanguage,
    /// The folders searched for the language files in search order.
    language_folders: Vec<PathBuf>,
}

impl ShellOptions {
    /// The shell settings for the game of `config` with the GUI font search
    /// paths and the language folders of `gui_resources`.
    pub(crate) fn for_config(config: &EmulatorConfig, gui_resources: GuiResources) -> Self {
        Self {
            backend: config.backend,
            audio_volume: config.audio_volume,
            window_size: WindowSize::Screen(initial_window_size(config.aspect_mode)),
            fullscreen: config.window_mode == WindowMode::Fullscreen,
            display_aspect_mode: graphics_display_aspect_mode(config.aspect_mode),
            large_native_target: wants_large_native_target(config),
            gui_font_paths: gui_resources.font_paths,
            gui_theme: config.gui_theme,
            gui_language: config.gui_language.clone(),
            language_folders: gui_resources.language_folders,
        }
    }

    /// The shell settings for the game manager: the backend, the volume, the
    /// window mode, the GUI font, the GUI theme and the GUI language of the
    /// settings file and the command line of `host`, and the language
    /// folders of `host`. A missing or invalid value keeps the built-in
    /// default.
    pub(crate) fn for_manager(host: &HostLayers) -> Self {
        let window_mode = host_setting(host, WINDOW_MODE_KEY, str::parse, DEFAULT_WINDOW_MODE);
        Self {
            backend: host_setting(host, BACKEND_KEY, str::parse, DEFAULT_BACKEND),
            audio_volume: host_setting(
                host,
                AUDIO_VOLUME_KEY,
                parse_audio_volume,
                DEFAULT_AUDIO_VOLUME,
            ),
            window_size: WindowSize::Logical(MANAGER_WINDOW_SIZE),
            fullscreen: window_mode == WindowMode::Fullscreen,
            display_aspect_mode: DisplayAspectMode::Aspect4By3,
            large_native_target: false,
            gui_font_paths: host.gui_font_paths(),
            gui_theme: host_setting(host, GUI_THEME_KEY, str::parse, DEFAULT_GUI_THEME),
            gui_language: host.gui_language(),
            language_folders: host.language_folders(),
        }
    }
}

/// Returns the value of `key` of `host` parsed by `parse`, or `default`
/// when the key is unset or its value is invalid.
fn host_setting<T, E: Display>(
    host: &HostLayers,
    key: &str,
    parse: impl Fn(&str) -> result::Result<T, E>,
    default: T,
) -> T {
    let Some(value) = host.host_value(key) else {
        return default;
    };
    parse(value).unwrap_or_else(|message| {
        warn!("Invalid {key}: {message}");
        default
    })
}

/// The process wide part of the application. It owns the game manager and
/// the session.
///
/// The field order is the drop order: the session flushes its media before
/// the engines, the window and SDL3 go.
pub(crate) struct Shell {
    /// The running game.
    session: Option<Session>,
    /// The game manager. `None` for a game of the command line.
    manager: Option<Manager>,
    /// The game that the manager asked to start.
    pending_launch: Option<Launch>,
    /// Whether the manager needs a new frame.
    frame_pending: bool,
    /// Whether the window can show a frame. It cannot while it is
    /// minimized, hidden or occluded.
    window_visible: bool,
    /// The stale manager frames in a row since the last window event.
    stale_frames: u32,
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
    /// the paused audio engine and the GUI host. The window fits the
    /// display. The manager and each session set the minimum size of the
    /// window.
    pub(crate) fn new(options: ShellOptions) -> Result<Self> {
        let SdlSubsystems {
            context: sdl_context,
            audio: audio_subsystem,
            video: video_subsystem,
            gamepad: gamepad_subsystem,
        } = initialize_sdl3()?;

        print_system_info();

        let usable = video_subsystem.primary_display_usable_size();
        let (requested, minimum) = match options.window_size {
            WindowSize::Screen(size) => (size, SESSION_MINIMUM_WINDOW_SIZE),
            WindowSize::Logical(size) => (size, MINIMUM_WINDOW_SIZE),
        };
        let (width, height) = fit_window_size(requested, usable, minimum);
        let mut window = build_window(&video_subsystem, width, height, options.fullscreen)?;
        if let WindowSize::Logical(size) = options.window_size {
            let factor =
                screen_units_per_logical_unit(window.display_scale(), window.pixel_density());
            let usable = window.display_usable_size().or(usable);
            let minimum = manager_minimum_size(factor, usable);
            let (width, height) = fit_window_size(logical_to_screen(size, factor), usable, minimum);
            if window.size() != (width, height) {
                if let Err(error) = window.set_size(width, height) {
                    warn!("Failed to set the window size {width} x {height}: {error}");
                }
                if let Err(error) = window.center() {
                    warn!("Failed to center the window: {error}");
                }
            }
        }

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
            gui_font_paths: options.gui_font_paths,
            gui_theme: theme_kind(options.gui_theme),
            gui_language: options.gui_language,
            language_folders: options.language_folders,
        });

        Ok(Self {
            session: None,
            manager: None,
            pending_launch: None,
            frame_pending: true,
            window_visible: true,
            stale_frames: 0,
            window_before_session: None,
            host,
            _video_subsystem: video_subsystem,
            sdl_context,
            should_quit: false,
            busy_duration: Duration::ZERO,
            window_title_last_update: Instant::now(),
        })
    }

    /// Keeps `manager` as the first screen and sets the minimum size of the
    /// manager window. Fails without the GUI font.
    pub(crate) fn open_manager(&mut self, manager: Manager) -> Result<()> {
        if let Some(message) = self.host.gui_error.as_ref() {
            return Err(Error::from(StringError(format!(
                "the game manager needs the GUI font: {message}"
            ))));
        }
        self.host.apply_manager_minimum_size();
        self.manager = Some(manager);
        Ok(())
    }

    /// Shows the window and runs the manager and the sessions until the
    /// application quits. A session of the command line ends the
    /// application. Returns an error when such a session ended with a panic.
    pub(crate) fn run(&mut self) -> Result<()> {
        self.host.window.show();

        let mut event_pump = self
            .sdl_context
            .event_pump()
            .context("Failed to get the SDL3 event pump")?;

        loop {
            if self.session.is_some() {
                self.run_session_loop(&mut event_pump);
                let exit = self.session.as_ref().and_then(Session::exit).cloned();
                let flush = self.end_session();
                let after = after_session(
                    exit.as_ref(),
                    self.should_quit,
                    self.manager.is_some(),
                    &flush,
                );
                match after {
                    AfterSession::Quit(result) => {
                        return result.map_err(|message| Error::from(StringError(message)));
                    }
                    AfterSession::Manager { panic } => {
                        self.restore_manager_window();
                        if let Some(manager) = self.manager.as_mut()
                            && let Some(layer) = manager.game_ended(panic.as_deref(), flush)
                            && let Some(gui) = self.host.gui.as_mut()
                        {
                            gui.open_layer(layer);
                        }
                    }
                }
                continue;
            }
            if self.manager.is_none() {
                return Ok(());
            }
            match self.run_manager_loop(&mut event_pump) {
                Some(launch) => self.launch(launch),
                None => return Ok(()),
            }
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

    /// Asks the manager for a new frame and starts the count of stale
    /// frames again.
    fn window_changed(&mut self) {
        self.frame_pending = true;
        self.stale_frames = 0;
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
    /// every other event to the session, or without a session to the
    /// manager.
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
                self.window_changed();
            }
            Event::Window {
                win_event: WindowEvent::Exposed | WindowEvent::Shown | WindowEvent::Restored,
                ..
            } => {
                self.window_visible = true;
                self.window_changed();
            }
            Event::Window {
                win_event: WindowEvent::Hidden | WindowEvent::Minimized | WindowEvent::Occluded,
                ..
            } => {
                self.window_visible = false;
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
                host.shift_held = false;
                if let Some(gui) = host.gui.as_mut() {
                    gui.gamepad.clear();
                    gui.release_left_button();
                }
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => {
                host.shift_held = keyboard::mod_state().shift();
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
                if self.session.is_none() {
                    host.apply_manager_minimum_size();
                }
                host.update_gui_window();
                self.window_changed();
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
                host.shift_held = shift_after(event, host.shift_held);
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
                } else if self.manager.is_some() {
                    self.handle_manager_input(event);
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
                .map(|_| ())
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
    use std::path::PathBuf;

    use common::temporary::TemporaryFolder;
    use sdl3_backend::DisplayAspectMode;

    use super::ShellOptions;
    use crate::{
        config::{
            AspectMode, Backend, EmulatorConfig, GuiLanguage, GuiResources, GuiTheme, HostLayers,
            LANGUAGE_FOLDER, LaunchRequest, Locations, SETTINGS_FILE_NAME, WindowMode,
            test_support::first_model_config,
        },
        host::{MANAGER_WINDOW_SIZE, WindowSize},
    };

    /// Returns the host layers of a root folder with a settings file that
    /// holds `settings`. Removes the folder before it returns.
    fn host_layers(name: &str, settings: &str) -> (HostLayers, PathBuf) {
        let folder = TemporaryFolder::new(&format!("shell_options_test_{name}"));
        let root = folder.path().to_path_buf();
        std::fs::write(root.join(SETTINGS_FILE_NAME), settings).expect("settings are written");
        let locations = Locations {
            current_directory: root.clone(),
            executable_directory: None,
            pref_path: None,
        };
        let host = HostLayers::read(&LaunchRequest::default(), &locations).expect("settings read");
        drop(folder);
        (host, root)
    }

    /// Verifies that the options of a game follow its window, engine, font, theme and language settings.
    #[test]
    fn options_follow_the_window_settings_of_the_game() {
        let config = EmulatorConfig {
            aspect_mode: AspectMode::Aspect1By1,
            window_mode: WindowMode::Fullscreen,
            backend: Backend::Legacy,
            audio_volume: 0.5,
            gui_theme: GuiTheme::Light,
            gui_language: GuiLanguage::Tag("ja".to_owned()),
            ..first_model_config()
        };
        let gui_font_paths = vec![PathBuf::from("/fonts/ui.ttf")];
        let language_folders = vec![PathBuf::from("/neetan/language")];
        let options = ShellOptions::for_config(
            &config,
            GuiResources {
                font_paths: gui_font_paths.clone(),
                language_folders: language_folders.clone(),
            },
        );
        assert_eq!(options.window_size, WindowSize::Screen((1280, 800)));
        assert!(options.fullscreen);
        assert_eq!(options.backend, Backend::Legacy);
        assert_eq!(options.audio_volume, 0.5);
        assert_eq!(options.display_aspect_mode, DisplayAspectMode::Aspect1By1);
        assert_eq!(options.gui_font_paths, gui_font_paths);
        assert_eq!(options.gui_theme, GuiTheme::Light);
        assert_eq!(options.gui_language, GuiLanguage::Tag("ja".to_owned()));
        assert_eq!(options.language_folders, language_folders);
    }

    /// Verifies that the manager window follows the window mode, the backend, the volume, the font, the theme and the language of the settings file.
    #[test]
    fn manager_options_follow_the_settings_file() {
        let (host, root) = host_layers(
            "settings",
            "window-mode = fullscreen\nbackend = legacy\naudio-volume = 0.25\ngui-font = fonts/ui.ttf\ngui-theme = light\ngui-language = pt-BR\n",
        );
        let options = ShellOptions::for_manager(&host);
        assert!(options.fullscreen);
        assert_eq!(options.backend, Backend::Legacy);
        assert_eq!(options.audio_volume, 0.25);
        assert_eq!(
            options.window_size,
            WindowSize::Logical(MANAGER_WINDOW_SIZE)
        );
        assert!(!options.large_native_target);
        assert_eq!(
            options.gui_font_paths,
            vec![root.join("fonts").join("ui.ttf")]
        );
        assert_eq!(options.gui_theme, GuiTheme::Light);
        assert_eq!(options.gui_language, GuiLanguage::Tag("pt-BR".to_owned()));
        assert_eq!(options.language_folders, vec![root.join(LANGUAGE_FOLDER)]);
    }

    /// Verifies that the manager keeps the built-in defaults for unset and invalid values.
    #[test]
    fn manager_options_keep_the_defaults_for_invalid_values() {
        let (host, root) = host_layers(
            "invalid",
            "backend = vulkan\naudio-volume = 7\ngui-theme = sepia\ngui-language = pt_BR\n",
        );
        let options = ShellOptions::for_manager(&host);
        assert_eq!(options.gui_theme, GuiTheme::Dark);
        assert_eq!(options.gui_language, GuiLanguage::Auto);
        let (unset, _) = host_layers("unset", "");
        assert_eq!(ShellOptions::for_manager(&unset).gui_theme, GuiTheme::Dark);
        assert!(!options.fullscreen);
        assert_eq!(options.backend, Backend::Modern);
        assert_eq!(options.audio_volume, 1.0);
        assert_eq!(
            options.gui_font_paths,
            vec![root.join(machine_factory::config::DEFAULT_GUI_FONT)]
        );
    }
}
