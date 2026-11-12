//! The overlay of a session over the paused game: the media dialog or the
//! help overlay.

use std::convert::Infallible;

use common::{JoystickState, warn};
use gui::component::{Status, run};

use super::Session;
use crate::{gui_host::GuiEvent, help_overlay::HelpOverlay, host::Host, media_dialog::MediaDialog};

/// The open overlay of a session. The session shows at most one.
pub(super) enum SessionOverlay {
    /// The media dialog of the removable drives.
    Media(Box<MediaDialog>),
    /// The list of the hotkeys.
    Help(Box<HelpOverlay>),
}

impl Session {
    /// Opens `overlay` over the game. With no overlay open, it pauses the
    /// emulation and the audio, and releases the relative mouse mode and the
    /// held joystick and mouse buttons of the machine. An open overlay gives
    /// way to `overlay` and the pause continues.
    pub(super) fn open_overlay(&mut self, overlay: SessionOverlay, host: &mut Host) {
        if self.overlay.is_none() {
            self.set_fast_forward(false, host);
            host.audio_engine.pause();
            if self.mouse_captured
                && let Err(error) = host.window.set_relative_mouse_mode(false)
            {
                warn!("Failed to release the relative mouse mode: {error}");
            }
            self.release_mouse_buttons();
            self.gamepad_buttons = JoystickState::default();
            self.gamepad_axis_x = 0;
            self.gamepad_axis_y = 0;
            self.keyboard_joystick = JoystickState::default();
            self.update_joystick(host);
        }
        if let Some(gui) = host.gui.as_mut() {
            gui.stop_text_input(Some(&host.window));
            gui.reset();
        }
        self.overlay = Some(overlay);
        host.update_gui_window();
        self.overlay_pass(host);
    }

    /// Closes the overlay. Restores the relative mouse mode and resumes the
    /// audio.
    pub(super) fn close_overlay(&mut self, host: &mut Host) {
        self.overlay = None;
        if let Some(gui) = host.gui.as_mut() {
            gui.stop_text_input(Some(&host.window));
            gui.gamepad.clear();
        }
        if self.mouse_captured
            && let Err(error) = host.window.set_relative_mouse_mode(true)
        {
            warn!("Failed to restore the relative mouse mode: {error}");
        }
        host.audio_engine.resume();
    }

    /// Adds `gui_event` to the input of the overlay and runs a pass when the
    /// event is discrete.
    pub(super) fn handle_gui_event(&mut self, gui_event: GuiEvent, host: &mut Host) {
        let Some(gui) = host.gui.as_mut() else {
            return;
        };
        if gui.apply(gui_event) {
            self.overlay_pass(host);
        }
    }

    /// Runs one pass of the open overlay.
    pub(super) fn overlay_pass(&mut self, host: &mut Host) {
        match self.overlay {
            Some(SessionOverlay::Media(_)) => self.media_pass(host),
            Some(SessionOverlay::Help(_)) => self.help_pass(host),
            None => {}
        }
    }

    /// Opens the help overlay, or closes it when it shows already. An open
    /// media dialog gives way to the help overlay.
    pub(super) fn help_hotkey(&mut self, host: &mut Host) {
        if !host.gui_available() {
            return;
        }
        if matches!(self.overlay, Some(SessionOverlay::Help(_))) {
            self.close_overlay(host);
            return;
        }
        let hotkeys = self.hotkey_features(host).hotkeys();
        self.open_overlay(
            SessionOverlay::Help(Box::new(HelpOverlay::new(hotkeys))),
            host,
        );
    }

    /// Runs one pass of the help overlay and closes it when the pass ends it.
    fn help_pass(&mut self, host: &mut Host) {
        let (Some(gui), Some(SessionOverlay::Help(help))) =
            (host.gui.as_mut(), self.overlay.as_mut())
        else {
            return;
        };
        let status = gui.pass(|context| {
            let area = context.bounds();
            run(
                help.as_mut(),
                context,
                area,
                (),
                &mut |never: Infallible| match never {},
            )
        });
        gui.sync_text_input(&host.window);
        match status {
            Status::Open => {}
            Status::Closed => self.close_overlay(host),
            Status::Done(never) => match never {},
        }
    }
}
