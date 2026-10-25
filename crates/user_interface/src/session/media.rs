//! The removable media of a session and its media dialog.

use common::{JoystickState, error, info, warn};

use super::Session;
use crate::{
    gui_host::GuiEvent,
    host::Host,
    media_dialog::{
        DialogAction, Drive, DriveMedia, HotkeyAction, MediaDialog, available_drives, hotkey_action,
    },
};

impl Session {
    /// Writes the floppy disks back and ejects the floppy of `drive`.
    pub(super) fn eject_floppy(&mut self, drive: usize) {
        self.flush_floppies_before_swap();
        self.machine.eject_floppy(drive);
        match drive {
            0 => self.fdd1_index = None,
            1 => self.fdd2_index = None,
            _ => {}
        }
        info!("Ejected FDD{}", drive + 1);
    }

    /// Writes the floppy disks back and inserts the configured floppy image
    /// `index` into `drive`.
    pub(super) fn select_floppy(&mut self, drive: usize, index: usize) -> Result<(), String> {
        let entries = match drive {
            0 => &self.fdd1_entries,
            1 => &self.fdd2_entries,
            _ => return Err(format!("floppy drive {} is not available", drive + 1)),
        };

        if index >= entries.len() {
            return Err(format!("floppy image index {index} is not available"));
        }

        let path = entries[index].path.clone();
        self.flush_floppies_before_swap();
        match self.machine.insert_floppy_from_path(drive, &path) {
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

    /// Ejects the CD-ROM.
    pub(super) fn eject_cdrom(&mut self) {
        self.machine.eject_cdrom();
        self.cdrom_index = None;
        info!("Ejected CD-ROM");
    }

    /// Inserts the configured CD-ROM image `index`.
    pub(super) fn select_cdrom(&mut self, index: usize) -> Result<(), String> {
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

    /// Ejects the cassette.
    pub(super) fn eject_cassette(&mut self) {
        self.machine.eject_cassette();
        info!("Ejected cassette");
    }

    /// Inserts the configured cassette.
    pub(super) fn select_cassette(&mut self) -> Result<(), String> {
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

    /// The removable media drives of the machine, in the tab order of the
    /// media dialog.
    fn media_drives(&self) -> Vec<Drive> {
        let capabilities = self.machine.startup_capabilities();
        available_drives(capabilities.floppy_drives, capabilities.cdrom)
    }

    /// The index of the loaded image of `drive`.
    fn loaded_image(&self, drive: Drive) -> Option<usize> {
        match drive {
            Drive::Floppy(0) => self.fdd1_index,
            Drive::Floppy(_) => self.fdd2_index,
            Drive::CdRom => self.cdrom_index,
        }
    }

    /// Opens the media dialog on the tab of `drive`, switches to that tab or
    /// closes the dialog when it shows that tab already.
    pub(super) fn media_hotkey(&mut self, drive: Drive, host: &mut Host) {
        if !host.gui_available() {
            return;
        }
        let drives = self.media_drives();
        let open_tab = self.media_dialog.as_ref().map(MediaDialog::tab);
        match hotkey_action(&drives, open_tab, drive) {
            HotkeyAction::Open(tab) => self.open_media_dialog(drives, tab, host),
            HotkeyAction::Switch(tab) => {
                let loaded = self.loaded_image(drive);
                if let Some(dialog) = self.media_dialog.as_mut() {
                    dialog.switch_tab(tab, loaded);
                }
                self.media_pass(host);
            }
            HotkeyAction::Close => self.close_media_dialog(host),
            HotkeyAction::Unavailable => {
                info!("This machine has no {} drive", drive.log_name());
            }
        }
    }

    /// Opens the media dialog over `drives` on the tab `tab`. Pauses the
    /// emulation and the audio, releases the relative mouse mode and the held
    /// joystick and mouse buttons of the machine.
    fn open_media_dialog(&mut self, drives: Vec<Drive>, tab: usize, host: &mut Host) {
        let loaded = self.loaded_image(drives[tab]);
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
        if let Some(gui) = host.gui.as_mut() {
            gui.reset();
        }
        self.media_dialog = Some(MediaDialog::new(drives, tab, loaded));
        host.update_gui_window();
        self.media_pass(host);
    }

    /// Closes the media dialog. Restores the relative mouse mode and resumes
    /// the audio.
    fn close_media_dialog(&mut self, host: &mut Host) {
        self.media_dialog = None;
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

    /// Adds `gui_event` to the input of the media dialog and runs a pass when
    /// the event is discrete.
    pub(super) fn handle_gui_event(&mut self, gui_event: GuiEvent, host: &mut Host) {
        let Some(gui) = host.gui.as_mut() else {
            return;
        };
        if gui.apply(gui_event) {
            self.media_pass(host);
        }
    }

    /// Runs one pass of the media dialog and applies the change it asks for.
    pub(super) fn media_pass(&mut self, host: &mut Host) {
        let (Some(gui), Some(dialog)) = (host.gui.as_mut(), self.media_dialog.as_mut()) else {
            return;
        };
        let media: Vec<DriveMedia> = dialog
            .drives()
            .iter()
            .map(|drive| match drive {
                Drive::Floppy(0) => DriveMedia {
                    entries: &self.fdd1_entries,
                    loaded: self.fdd1_index,
                },
                Drive::Floppy(_) => DriveMedia {
                    entries: &self.fdd2_entries,
                    loaded: self.fdd2_index,
                },
                Drive::CdRom => DriveMedia {
                    entries: &self.cdrom_entries,
                    loaded: self.cdrom_index,
                },
            })
            .collect();
        let mut action = gui.pass(|context| dialog.show(context, &media));
        while action == DialogAction::None && dialog.take_stale() {
            action = gui.pass(|context| dialog.show(context, &media));
        }
        gui.sync_text_input(&host.window);
        match action {
            DialogAction::None => {}
            DialogAction::Close => self.close_media_dialog(host),
            DialogAction::Insert { drive, index } => {
                let _ = match drive {
                    Drive::Floppy(number) => self.select_floppy(number, index),
                    Drive::CdRom => self.select_cdrom(index),
                };
                self.close_media_dialog(host);
            }
            DialogAction::Eject { drive } => {
                match drive {
                    Drive::Floppy(number) => self.eject_floppy(number),
                    Drive::CdRom => self.eject_cdrom(),
                }
                self.close_media_dialog(host);
            }
        }
    }
}
