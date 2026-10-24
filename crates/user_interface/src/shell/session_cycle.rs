//! The start and the end of a session: the creation of the machine, the
//! window and engine settings of the game, the flush and the step after the
//! session.

use std::time::{Duration, Instant};

use common::{Context, Error, FlushError, StringError, error, info, warn};

use super::Shell;
use crate::{
    GAME_NAME, Result,
    config::{Backend, EmulatorConfig, WindowMode},
    host::{
        SESSION_MINIMUM_WINDOW_SIZE, aspect_ratio_for_mode, fit_window_size, graphics_scaling,
        initial_window_size,
    },
    input::KeyOverrides,
    manager::Launch,
    on_off,
    session::{LaunchOrigin, Session, SessionExit, catch_panic},
};

/// The step of the application after a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AfterSession {
    /// The game manager returns. It shows the message of a core that panicked.
    Manager {
        /// The panic message of the core.
        panic: Option<String>,
    },
    /// The application quits. An error makes the process exit with code 1.
    Quit(std::result::Result<(), String>),
}

/// Returns the step after a session that ended with `exit` and whose
/// final flush gave `flush`.
///
/// A session of the manager returns to the manager unless the user closed
/// the window. A session of the command line quits the application. Its
/// panic and its failed flush become the error of the application.
pub(super) fn after_session(
    exit: Option<&SessionExit>,
    quit_requested: bool,
    has_manager: bool,
    flush: &std::result::Result<(), FlushError>,
) -> AfterSession {
    let panic = match exit {
        Some(SessionExit::Panic { message }) => Some(message.clone()),
        Some(SessionExit::UserExit | SessionExit::GuestShutdown) | None => None,
    };
    if has_manager {
        if quit_requested {
            return AfterSession::Quit(Ok(()));
        }
        return AfterSession::Manager { panic };
    }
    let mut errors = Vec::new();
    if let Some(message) = panic {
        errors.push(format!(
            "the session ended after a panic of the emulator: {message}"
        ));
    }
    if let Err(error) = flush {
        errors.push(format!("the disk images were not saved: {error}"));
    }
    if errors.is_empty() {
        AfterSession::Quit(Ok(()))
    } else {
        AfterSession::Quit(Err(errors.join("\n")))
    }
}

impl Shell {
    /// Creates the session of `config` and applies its window and display
    /// settings. A panic while the machine starts becomes an error.
    pub(crate) fn start_session(
        &mut self,
        config: EmulatorConfig,
        key_overrides: KeyOverrides,
    ) -> Result<()> {
        if let Some(message) = self.host.gui_error.as_ref() {
            warn!("The media dialog is unavailable: {message}");
        }
        let session = self
            .create_session(config, key_overrides, LaunchOrigin::CommandLine)
            .map_err(|message| Error::from(StringError(message)))?;
        self.begin_session(session)
    }

    /// Creates the session of `config` started from `origin`. Returns the
    /// message of an error or of a panic while the machine starts.
    fn create_session(
        &self,
        config: EmulatorConfig,
        key_overrides: KeyOverrides,
        origin: LaunchOrigin,
    ) -> std::result::Result<Session, String> {
        let backend = self.host.backend;
        match catch_panic(|| Session::new(config, key_overrides, backend, origin)) {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(error)) => Err(format!("{error:#}")),
            Err(message) => Err(format!("the machine panicked while starting: {message}")),
        }
    }

    /// Starts the game of `launch` from the manager. An error of the
    /// configuration or of the machine opens the error dialog of the manager.
    pub(super) fn launch(&mut self, launch: Launch) {
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        info!("Starting {} ({})", launch.title, launch.system.label);
        let configuration = manager
            .configuration(&launch)
            .map_err(|error| format!("{error:#}"));
        let session = configuration.and_then(|(config, key_overrides)| {
            self.create_session(config, key_overrides, LaunchOrigin::Manager)
        });
        let result = match session {
            Ok(session) => self
                .begin_session(session)
                .map_err(|error| format!("{error:#}")),
            Err(message) => Err(message),
        };
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        match result {
            Ok(()) => manager.game_started(launch),
            Err(message) => {
                error!("Could not start {}: {message}", launch.title);
                let layer = manager.launch_failed(&launch, &message);
                if let Some(gui) = self.host.gui.as_mut() {
                    gui.open_layer(layer);
                }
            }
        }
    }

    /// Applies the display aspect mode and the native target of `session`
    /// to the graphics engine, records the window state, applies the
    /// minimum size of a session window, the window mode, the window size
    /// and the aspect ratio lock of the game, resumes the audio and keeps
    /// the session. Fails when the graphics engine cannot rebuild its
    /// native target.
    fn begin_session(&mut self, session: Session) -> Result<()> {
        let host = &mut self.host;
        host.graphics_engine
            .set_display_aspect_mode(session.display_aspect_mode());
        host.graphics_engine
            .set_large_native_target(session.large_native_target())
            .context("Failed to resize the native target")?;
        self.window_before_session = Some(host.window_state());
        host.apply_session_minimum_size();
        let fullscreen = session.window_mode() == WindowMode::Fullscreen;
        if fullscreen != host.fullscreen {
            host.set_fullscreen(fullscreen);
        }
        if !fullscreen {
            let aspect_ratio = aspect_ratio_for_mode(session.aspect_mode());
            if let Err(error) = host.window.set_aspect_ratio(aspect_ratio) {
                warn!("Failed to lock window aspect ratio to {aspect_ratio}: {error}");
            }
            let (width, height) = fit_window_size(
                initial_window_size(session.aspect_mode()),
                host.window.display_usable_size(),
                SESSION_MINIMUM_WINDOW_SIZE,
            );
            if host.window.size() != (width, height)
                && let Err(error) = host.window.set_size(width, height)
            {
                warn!("Failed to set the window size {width} x {height}: {error}");
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
        Ok(())
    }

    /// Ends the session: flushes and drops it. Returns the errors of the
    /// flush.
    pub(super) fn end_session(&mut self) -> std::result::Result<(), FlushError> {
        let Some(mut session) = self.session.take() else {
            return Ok(());
        };
        let flush = session.flush_media();
        if let Err(message) = catch_panic(move || drop(session)) {
            error!("Closing the session panicked: {message}");
        }
        flush
    }

    /// Prepares the window for the manager after a session: releases the
    /// mouse capture, clears the aspect ratio lock, sets the minimum size of
    /// the manager window, restores the window size and fullscreen state of
    /// the time before the session, waits until the window system applied
    /// it, stops the audio, resets the GUI host and restores the window
    /// title.
    pub(super) fn restore_manager_window(&mut self) {
        let host = &mut self.host;
        if let Err(error) = host.window.set_relative_mouse_mode(false) {
            warn!("Failed to release the relative mouse mode: {error}");
        }
        if let Err(error) = host.window.clear_aspect_ratio() {
            warn!("Failed to clear the window aspect ratio lock: {error}");
        }
        host.apply_manager_minimum_size();
        if let Some(state) = self.window_before_session.take() {
            host.restore_window_state(state);
        }
        if !host.window.sync() {
            warn!("The window system did not apply the window state of the manager in time");
        }
        host.audio_engine.stop();
        if let Some(gui) = host.gui.as_mut() {
            gui.stop_text_input(Some(&host.window));
            gui.reset();
        }
        host.window.set_title(GAME_NAME);
        self.busy_duration = Duration::ZERO;
        self.window_title_last_update = Instant::now();
        self.window_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panic of the test sessions.
    fn panic_exit() -> SessionExit {
        SessionExit::Panic {
            message: "core halted".to_owned(),
        }
    }

    /// The failed flush of the test sessions.
    fn failed_flush() -> std::result::Result<(), FlushError> {
        Err(FlushError::new("floppy image /a.d88 could not be written"))
    }

    /// Verifies the step after a session of the manager for every exit, flush result and a closed window.
    #[test]
    fn a_session_of_the_manager_returns_unless_the_window_closed() {
        for flush in [Ok(()), failed_flush()] {
            for exit in [
                None,
                Some(SessionExit::UserExit),
                Some(SessionExit::GuestShutdown),
            ] {
                assert_eq!(
                    after_session(exit.as_ref(), false, true, &flush),
                    AfterSession::Manager { panic: None }
                );
                assert_eq!(
                    after_session(exit.as_ref(), true, true, &flush),
                    AfterSession::Quit(Ok(()))
                );
            }
            assert_eq!(
                after_session(Some(&panic_exit()), false, true, &flush),
                AfterSession::Manager {
                    panic: Some("core halted".to_owned())
                }
            );
            assert_eq!(
                after_session(Some(&panic_exit()), true, true, &flush),
                AfterSession::Quit(Ok(()))
            );
        }
    }

    /// Verifies that a session of the command line quits and that its panic and its failed flush are errors.
    #[test]
    fn a_session_of_the_command_line_quits_and_reports_its_errors() {
        let panic_error = "the session ended after a panic of the emulator: core halted";
        let flush_error =
            "the disk images were not saved: floppy image /a.d88 could not be written";
        for quit_requested in [false, true] {
            for exit in [
                None,
                Some(SessionExit::UserExit),
                Some(SessionExit::GuestShutdown),
            ] {
                assert_eq!(
                    after_session(exit.as_ref(), quit_requested, false, &Ok(())),
                    AfterSession::Quit(Ok(()))
                );
                assert_eq!(
                    after_session(exit.as_ref(), quit_requested, false, &failed_flush()),
                    AfterSession::Quit(Err(flush_error.to_owned()))
                );
            }
            assert_eq!(
                after_session(Some(&panic_exit()), quit_requested, false, &Ok(())),
                AfterSession::Quit(Err(panic_error.to_owned()))
            );
            assert_eq!(
                after_session(Some(&panic_exit()), quit_requested, false, &failed_flush()),
                AfterSession::Quit(Err(format!("{panic_error}\n{flush_error}")))
            );
        }
    }
}
